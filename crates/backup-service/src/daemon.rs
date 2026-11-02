use backup_core::*;
use backup_service::capabilities;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use zbus::{Connection, message::Header, object_server::SignalEmitter};
struct Api {
    store: Store,
    active: Arc<Mutex<Option<(String, Arc<AtomicBool>)>>>,
}
fn failure(e: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}
async fn authorize(c: &Connection, h: &Header<'_>) -> zbus::fdo::Result<()> {
    let sender = h
        .sender()
        .ok_or_else(|| zbus::fdo::Error::AccessDenied("Missing sender".into()))?;
    let uid = zbus::fdo::DBusProxy::new(c)
        .await?
        .get_connection_unix_user(sender.clone().into())
        .await?;
    if uid != unsafe { libc::geteuid() } {
        return Err(zbus::fdo::Error::AccessDenied(
            "Same-user access required".into(),
        ));
    }
    Ok(())
}
#[zbus::interface(name = "io.github.design_nexus.nyx.Backup1")]
impl Api {
    async fn capabilities(
        &self,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        Ok(capabilities().to_string())
    }
    async fn status(
        &self,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        let mut health = health(&self.store).map_err(failure)?;
        health.next_schedule = backup_system::schedules()
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(serde_json::to_string(&health).map_err(failure)?)
    }
    async fn destinations(
        &self,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        let config = self.store.load().map_err(failure)?;
        let v:Vec<_>=config.destinations.iter().map(|d|serde_json::json!({"id":d.id,"name":d.name,"mounted_destination_available":backup_system::validate_destination(d).is_ok()})).collect();
        Ok(serde_json::json!({"version":1,"destinations":v}).to_string())
    }
    async fn jobs(
        &self,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        Ok(serde_json::to_string(&self.store.jobs().map_err(failure)?).map_err(failure)?)
    }
    async fn job(
        &self,
        id: &str,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        Ok(serde_json::to_string(&self.store.job(id).map_err(failure)?).map_err(failure)?)
    }
    async fn preview(
        &self,
        id: &str,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        let store = self.store.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let config = store.load()?;
            let p = config.profile(&id)?;
            Ok(serde_json::to_string(
                &backup_engine::select(&store, &config, p)?.preview,
            )?)
        })
        .await
        .map_err(failure)?
        .map_err(failure)
    }
    async fn start(
        &self,
        id: &str,
        operation: &str,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        if !["backup", "verify", "verify-full", "history", "stats"].contains(&operation) {
            return Err(zbus::fdo::Error::AccessDenied(
                "Operation is not exposed by this API".into(),
            ));
        }
        let config = self.store.load().map_err(failure)?;
        let p = config.profile(id).map_err(failure)?.clone();
        if !p.approved {
            return Err(zbus::fdo::Error::AccessDenied(
                "Approve this profile in Backup before starting jobs".into(),
            ));
        }
        let mut active = self.active.lock().map_err(failure)?;
        if active.is_some() {
            return Err(failure("A repository operation is already running"));
        }
        // Fail before publishing a running job when a local administration operation owns the worker.
        let check = backup_engine::RepoLock::acquire(&self.store).map_err(failure)?;
        drop(check);
        let mut job = Job::new(id, operation);
        let job_id = job.id.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.store.write_job(&job).map_err(failure)?;
        *active = Some((job_id.clone(), cancel.clone()));
        drop(active);
        let store = self.store.clone();
        let active = self.active.clone();
        let c = c.clone();
        let operation = operation.to_string();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let emit = |job: &Job| {
                if let Ok(json) = serde_json::to_string(job) {
                    let _ = handle.block_on(async {
                        let emitter = SignalEmitter::new(&c, OBJECT)?;
                        Api::job_changed(&emitter, &json).await
                    });
                }
            };
            emit(&job);
            if let Err(e) =
                backup_engine::execute(&store, &config, &p, &operation, cancel, &mut job, emit)
            {
                job.state = JobState::Failed;
                job.finished = Some(now());
                job.message = e.to_string();
                let _ = store.write_job(&job);
                emit(&job);
            }
            if let Ok(mut a) = active.lock() {
                *a = None;
            }
            if config.notifications && operation == "backup" {
                backup_system::notify(job.state == JobState::Success);
            }
        });
        Ok(job_id)
    }
    async fn cancel(
        &self,
        id: &str,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        let active = self.active.lock().map_err(failure)?;
        let Some((active_id, cancel)) = active.as_ref() else {
            return Err(failure("No active job"));
        };
        if active_id != id {
            return Err(failure("This job is not active"));
        }
        cancel.store(true, Ordering::SeqCst);
        Ok("Cancellation requested".into())
    }
    #[zbus(signal)]
    async fn job_changed(emitter: &SignalEmitter<'_>, job: &str) -> zbus::Result<()>;
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    backup_core::init();
    let store = Store::default();
    backup_engine::reconcile(&store)?;
    let active = Arc::new(Mutex::new(None));
    let _connection = zbus::connection::Builder::session()?
        .name(BUS)?
        .serve_at(OBJECT, Api { store, active })?
        .build()
        .await?;
    // systemd owns the service lifetime. Its control group also owns all workers.
    std::future::pending::<()>().await;
    Ok(())
}
