//! Polls the background service. Bus calls run off the GTK thread.
use crate::ui::*;
use backup_core::*;
use gtk::glib;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

thread_local! {
    static POLLING: Cell<bool> = const { Cell::new(false) };
    static ACTIVE: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn start(ui: &Rc<Ui>) {
    poll(ui);
    let weak = Rc::downgrade(ui);
    glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        let Some(ui) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        poll(&ui);
        glib::ControlFlow::Continue
    });
}

/// Poll now (a job was just started), not at the next tick.
pub fn poll_soon(ui: &Rc<Ui>) {
    let ui = ui.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || poll(&ui));
}

fn poll(ui: &Rc<Ui>) {
    if POLLING.with(|p| p.replace(true)) {
        return;
    }
    let ui = ui.clone();
    attempt(
        || {
            let s = backup_service::status()?;
            Ok(serde_json::from_str::<Health>(&s)?)
        },
        move |result| {
            POLLING.with(|p| p.set(false));
            match result {
                Ok(h) => apply(&ui, h),
                Err(_) => {
                    *ui.health.borrow_mut() = None;
                    crate::backups::render_status(&ui, false);
                }
            }
        },
    );
}

fn apply(ui: &Rc<Ui>, health: Health) {
    let active = health.active.as_ref().map(|j| j.id.clone());
    let finished = ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let finished = a.is_some() && *a != active;
        let previous = a.clone();
        *a = active.clone();
        finished.then_some(previous).flatten()
    });
    *ui.health.borrow_mut() = Some(health);
    crate::backups::render_status(ui, true);
    if ui.view() == "activity" && active.is_some() {
        crate::activity::refresh(ui);
    }
    if let Some(id) = finished {
        crate::backups::refresh(ui);
        crate::activity::refresh(ui);
        announce(ui, id);
    }
}

/// Say how a backup that finished while the window was open went.
fn announce(ui: &Rc<Ui>, id: String) {
    let ui = ui.clone();
    attempt(
        move || {
            let s = backup_service::client_call("Job", (id.as_str(),))?;
            Ok(serde_json::from_str::<Job>(&s)?)
        },
        move |result| {
            let Ok(job) = result else { return };
            if job.operation != "backup" {
                return;
            }
            let name = profile_name(&job.profile);
            toast(
                &ui,
                &match job.state {
                    JobState::Success => format!("{name} is backed up"),
                    JobState::Cancelled => format!("Backup of {name} cancelled"),
                    JobState::Incomplete => {
                        format!("{name} backed up, but some files couldn't be read")
                    }
                    _ => format!("Backup of {name} needs attention"),
                },
            );
        },
    );
}
