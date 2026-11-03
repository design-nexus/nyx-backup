use anyhow::Context;
use backup_core::*;
use backup_service::capabilities;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use zbus::{Connection, message::Header, object_server::SignalEmitter};
#[derive(Clone)]
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
impl Api {
    fn launch(
        &self,
        id: &str,
        operation: &str,
        c: &Connection,
        require_approved: bool,
        restore: Option<backup_restore::Request>,
        migration: Option<backup_migrate::Request>,
    ) -> zbus::fdo::Result<String> {
        let config = self.store.load().map_err(failure)?;
        let p = config.profile(id).map_err(failure)?.clone();
        if require_approved && !p.approved {
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
        let mut job = Job::new(id, operation);
        job.progress = serde_json::json!({"stage":"preparing"});
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
            let result = if let Some(request) = migration {
                backup_migrate::execute_locked(
                    check, &store, &config, &p, request, cancel, &mut job, emit,
                )
            } else {
                match restore {
                    Some(request) => backup_restore::execute_locked(
                        check, &store, &config, &p, request, cancel, &mut job, emit,
                    ),
                    None => backup_engine::execute_locked(
                        check, &store, &config, &p, &operation, cancel, &mut job, emit,
                    ),
                }
            };
            if let Err(e) = result {
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
        let _ = (id, operation);
        Err(zbus::fdo::Error::AccessDenied("Use the granted Core Control interface; explicit local clients use the private channel".into()))
    }
    async fn cancel(
        &self,
        id: &str,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        let _ = id;
        Err(zbus::fdo::Error::AccessDenied(
            "Use granted backup.cancel or the explicit private local client".into(),
        ))
    }
    #[zbus(signal)]
    async fn job_changed(emitter: &SignalEmitter<'_>, job: &str) -> zbus::Result<()>;
}
// Local GUI/CLI administration is separate from the integration D-Bus API.
// The kernel supplies peer credentials; neither requests nor replies contain secrets.
async fn handle_admin(
    api: &Api,
    c: &Connection,
    stream: tokio::net::UnixStream,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    anyhow::ensure!(
        stream.peer_cred()?.uid() == unsafe { libc::geteuid() },
        "Same-user access required"
    );
    let mut reader = tokio::io::BufReader::new(stream.take(8 * 1024 * 1024));
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_line(&mut line),
    )
    .await??;
    let result = async {
        let request: backup_service::AdminRequest = serde_json::from_str(&line)?;
        request.validate()?;
        anyhow::ensure!(
            request.configuration == backup_service::configuration_file(&api.store),
            "Client and service use different configurations; refusing repository administration"
        );
        if request.operation == "cancel" {
            let job = request.job.as_ref().context("Missing job")?;
            anyhow::ensure!(
                api.store.job(job)?.profile == request.profile,
                "Job belongs to another profile"
            );
            let active = api
                .active
                .lock()
                .map_err(|_| anyhow::anyhow!("Worker unavailable"))?;
            let Some((id, cancel)) = active.as_ref() else {
                anyhow::bail!("No active job")
            };
            anyhow::ensure!(id == job, "Job is not active");
            cancel.store(true, Ordering::SeqCst);
            return Ok(serde_json::json!({"version":1,"cancel_requested":job}));
        }
        if let Some(migration) = &request.migration {
            if !migration.is_job() {
                let store = api.store.clone();
                let profile = request.profile.clone();
                let migration = migration.clone();
                let value =
                    tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
                        let config = store.load()?;
                        let p = config.profile(&profile)?;
                        backup_migrate::read(&store, &config, p, &migration)
                    })
                    .await??;
                return Ok::<_, anyhow::Error>(serde_json::json!({"version":1,"result":value}));
            }
        }
        if let Some(restore) = &request.restore {
            if !restore.is_job() {
                let store = api.store.clone();
                let profile = request.profile.clone();
                let restore = restore.clone();
                let value =
                    tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
                        let config = store.load()?;
                        let p = config.profile(&profile)?;
                        backup_restore::read(&store, &config, p, &restore)
                    })
                    .await??;
                return Ok::<_, anyhow::Error>(serde_json::json!({"version":1,"result":value}));
            }
        }
        let id = api.launch(
            &request.profile,
            &request.operation,
            c,
            ["backup", "verify", "verify-full", "history", "stats"]
                .contains(&request.operation.as_str()),
            request.restore,
            request.migration,
        )?;
        Ok(serde_json::json!({"version":1,"job":id}))
    }
    .await;
    let response = match result {
        Ok(value) => value,
        Err(e) => serde_json::json!({"version":1,"error":e.to_string()}),
    };
    let response = response.to_string();
    let response = if response.len() > 16 * 1024 * 1024 {
        serde_json::json!({"version":1,"error":"Restore result too large; browse a smaller folder"})
            .to_string()
    } else {
        response
    };
    let mut stream = reader.into_inner().into_inner();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.write_all(format!("{response}\n").as_bytes()),
    )
    .await??;
    Ok(())
}
#[derive(Clone)]
struct Control {
    api: Api,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlArgs {
    grant: String,
    #[serde(default)]
    job: Option<String>,
}
#[zbus::interface(name = "io.github.design_nexus.nyx.Control")]
impl Control {
    async fn describe(
        &self,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        let owner = h
            .sender()
            .ok_or_else(|| failure("Missing sender"))?
            .as_str();
        let config = self.api.store.load().map_err(failure)?;
        let mut grants: Vec<_> = backup_service::integration::list(&self.api.store)
            .map_err(failure)?
            .into_iter()
            .filter(|g| {
                g.owner == owner
                    && g.expires > now()
                    && config.profile(&g.profile).is_ok_and(|p| p.approved)
            })
            .collect();
        grants.sort_by_key(|g| std::cmp::Reverse(g.expires));
        let mut tools = Vec::new();
        for (name, action, description) in [
            (
                "backup.status",
                "status",
                "Granted profile health and progress",
            ),
            (
                "backup.start",
                "backup",
                "Start the granted approved profile",
            ),
            ("backup.verify", "verify", "Verify the granted profile"),
            (
                "backup.cancel",
                "cancel",
                "Cancel the granted profile's active job",
            ),
        ] {
            let ids: Vec<_> = grants
                .iter()
                .filter(|g| g.actions.iter().any(|a| a == action))
                .map(|g| g.id.clone())
                .collect();
            if ids.is_empty() {
                continue;
            }
            let mut schema = serde_json::json!({"type":"object","additionalProperties":false,"properties":{"grant":{"type":"string","enum":ids}},"required":["grant"]});
            if action == "cancel" {
                schema["properties"]["job"] = serde_json::json!({"type":"string"});
                schema["required"] = serde_json::json!(["grant", "job"]);
            }
            tools.push(
                serde_json::json!({"name":name,"description":description,"inputSchema":schema}),
            );
        }
        Ok(serde_json::to_string(&tools).map_err(failure)?)
    }
    async fn call(
        &self,
        name: &str,
        arguments: &str,
        #[zbus(connection)] c: &Connection,
        #[zbus(header)] h: Header<'_>,
    ) -> zbus::fdo::Result<String> {
        authorize(c, &h).await?;
        let result = async {
            anyhow::ensure!(arguments.len() <= 4096, "Control request is too large");
            let a: ControlArgs = serde_json::from_str(arguments)?;
            let owner = h
                .sender()
                .ok_or_else(|| anyhow::anyhow!("Missing sender"))?
                .as_str();
            let action = match name {
                "backup.status" => "status",
                "backup.start" => "backup",
                "backup.verify" => "verify",
                "backup.cancel" => "cancel",
                _ => anyhow::bail!("Unsupported control action"),
            };
            let g = backup_service::integration::check(&self.api.store, &a.grant, owner, action)?;
            let bus = zbus::Proxy::new(
                c,
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
            )
            .await?;
            let current: String = bus.call("GetNameOwner", &(&g.consumer,)).await?;
            anyhow::ensure!(
                current == owner,
                "Consumer bus-name ownership changed; grant a new connection"
            );
            match action {
                "status" => {
                    anyhow::ensure!(a.job.is_none(), "Status does not accept a job");
                    backup_service::integration::summary(&self.api.store, &g)
                }
                "cancel" => {
                    let id = a.job.context("Choose a job to cancel")?;
                    anyhow::ensure!(
                        self.api.store.job(&id)?.profile == g.profile,
                        "Job belongs to another profile"
                    );
                    let active = self
                        .api
                        .active
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Worker state unavailable"))?;
                    let Some((job, cancel)) = active.as_ref() else {
                        anyhow::bail!("No active job")
                    };
                    anyhow::ensure!(job == &id, "Job is not active");
                    cancel.store(true, Ordering::SeqCst);
                    Ok(serde_json::json!({"cancel_requested":id}))
                }
                operation => {
                    anyhow::ensure!(a.job.is_none(), "Start does not accept a job");
                    let id = self
                        .api
                        .launch(&g.profile, operation, c, true, None, None)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    Ok(serde_json::json!({"job":id}))
                }
            }
        }
        .await;
        Ok(match result {
            Ok(v) => serde_json::json!({"ok":v}),
            Err(e) => serde_json::json!({"error":e.to_string()}),
        }
        .to_string())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    backup_core::init();
    let store = Store::default();
    backup_engine::reconcile(&store)?;
    let active = Arc::new(Mutex::new(None));
    let api = Api { store, active };
    let connection = zbus::connection::Builder::session()
        .name(BUS)
        .serve_at(OBJECT, api.clone())
        .serve_at(OBJECT, Control { api: api.clone() })
        .build()
        .await?;
    let socket = backup_service::admin_socket(&api.store);
    backup_core::private_dir(socket.parent().unwrap())?;
    if let Ok(metadata) = std::fs::symlink_metadata(&socket) {
        use std::os::unix::fs::FileTypeExt;
        anyhow::ensure!(
            metadata.file_type().is_socket(),
            "Administration socket path is occupied by another file"
        );
        match std::os::unix::net::UnixStream::connect(&socket) {
            Ok(_) => anyhow::bail!("An administration listener is already active"),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                ) =>
            {
                std::fs::remove_file(&socket)?
            }
            Err(e) => return Err(e.into()),
        }
    }
    let listener = tokio::net::UnixListener::bind(&socket)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let api = api.clone();
            let connection = connection.clone();
            tokio::spawn(async move {
                let _ = handle_admin(&api, &connection, stream).await;
            });
        }
    });
    // systemd owns the service lifetime. Its control group also owns all workers.
    std::future::pending::<()>().await;
    Ok(())
}
