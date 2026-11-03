//! The window's shared state and the small helpers every view uses.
use backup_core::*;
use gtk::{glib, pango, prelude::*};
use nyx_ui::widgets as w;
use std::{cell::RefCell, rc::Rc};

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    pub overlay: gtk::Overlay,
    pub header: nyx_ui::chrome::AppHeader,
    pub views: gtk::Stack,
    pub switch: gtk::Box,
    pub actions: gtk::Stack,
    pub backups: crate::backups::View,
    pub restore: crate::restore_ui::View,
    pub activity: crate::activity::View,
    pub settings: crate::settings::Page,
    pub health: RefCell<Option<Health>>,
    pub toast: RefCell<Option<gtk::Widget>>,
}

pub const VIEWS: &[(&str, &str)] = &[
    ("backups", "Backups"),
    ("restore", "Restore"),
    ("activity", "Activity"),
];

impl Ui {
    /// Show a view and press its segment in the header.
    pub fn show(&self, view: &str) {
        let index = VIEWS.iter().position(|(id, _)| *id == view).unwrap_or(0);
        let mut child = self.switch.first_child();
        let mut i = 0;
        while let Some(c) = child {
            if i == index {
                if let Some(b) = c.downcast_ref::<gtk::ToggleButton>() {
                    b.set_active(true);
                }
            }
            i += 1;
            child = c.next_sibling();
        }
        self.views.set_visible_child_name(view);
        self.actions.set_visible_child_name(view);
    }
    pub fn view(&self) -> String {
        self.views
            .visible_child_name()
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}

pub fn label(text: &str, class: &str) -> gtk::Label {
    let l = w::label(text, class);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(pango::WrapMode::WordChar);
    l
}

/// A one-line label that ellipsizes instead of wrapping.
pub fn line(text: &str, class: &str) -> gtk::Label {
    let l = w::label(text, class);
    l.set_xalign(0.0);
    l.set_ellipsize(pango::EllipsizeMode::End);
    l
}

pub fn clear(b: &impl IsA<gtk::Widget>) {
    let b = b.upcast_ref::<gtk::Widget>();
    while let Some(c) = b.first_child() {
        if let Some(list) = b.downcast_ref::<gtk::ListBox>() {
            list.remove(&c);
        } else if let Some(bx) = b.downcast_ref::<gtk::Box>() {
            bx.remove(&c);
        } else {
            c.unparent();
        }
    }
}

pub fn icon_button(icon: &str, tip: &str, run: impl Fn() + 'static) -> gtk::Button {
    let b = w::icon_button(icon, tip);
    b.connect_clicked(move |_| run());
    b
}

/// "Today 09:12", "Yesterday 21:00", "Mon, Nov 3 09:12" or "Nov 3, 2025".
pub fn when(secs: u64) -> String {
    let Ok(t) = glib::DateTime::from_unix_local(secs as i64) else {
        return "Unknown time".into();
    };
    let Ok(now) = glib::DateTime::now_local() else {
        return "Unknown time".into();
    };
    let day = |d: &glib::DateTime| (d.year(), d.day_of_year());
    let format = if day(&t) == day(&now) {
        "Today %H:%M".to_string()
    } else if now.add_days(-1).is_ok_and(|y| day(&y) == day(&t))
        || now.add_days(1).is_ok_and(|y| day(&y) == day(&t))
    {
        if t > now {
            "Tomorrow %H:%M".into()
        } else {
            "Yesterday %H:%M".into()
        }
    } else if t.year() == now.year() {
        "%a, %b %-e %H:%M".into()
    } else {
        "%b %-e, %Y".into()
    };
    t.format(&format)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "Unknown time".into())
}

/// Restic's RFC 3339 snapshot time as `when`.
pub fn when_text(time: &str) -> String {
    glib::DateTime::from_iso8601(time, None)
        .map(|d| when(d.to_unix() as u64))
        .unwrap_or_else(|_| time.to_string())
}

pub fn size(bytes: u64) -> String {
    glib::format_size(bytes).to_string()
}

/// 3412 → "3,412".
pub fn number(n: u64) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    grouped
}

pub fn count(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", number(n), if n == 1 { one } else { many })
}

pub fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        _ => format!("{} h {} min", secs / 3600, secs / 60 % 60),
    }
}

/// What a job's operation is called in the interface.
pub fn operation_name(operation: &str) -> &'static str {
    match operation {
        "backup" => "Backup",
        "verify" => "Repository check",
        "verify-full" => "Full data check",
        "history" => "Snapshot list",
        "stats" => "Storage use",
        "validate" => "Repository validation",
        "init" => "Repository creation",
        "retention-preview" => "Retention preview",
        "retention-apply" => "Snapshot cleanup",
        "restore-plan" => "Restore preparation",
        "restore-apply" => "Restore",
        "restore-rollback" => "Rollback",
        "migration-prepare" => "Migration preparation",
        _ => "Operation",
    }
}

pub fn profile_name(id: &str) -> String {
    Store::default()
        .load()
        .ok()
        .and_then(|c| c.profile(id).ok().map(|p| p.name.clone()))
        .unwrap_or_else(|| id.to_string())
}

/// Run `job` off the GTK thread; errors (and panics) open an error card.
pub fn work<T: Send + 'static>(
    ui: &Rc<Ui>,
    job: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    done: impl FnOnce(&Rc<Ui>, T) + 'static,
) {
    let ui = ui.clone();
    nyx_core::cmd::background(
        move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "The request stopped unexpectedly. Try again."
                ))
            })
        },
        move |result| match result {
            Ok(v) => done(&ui, v),
            Err(e) => error(&ui, "Something needs attention", &e.to_string()),
        },
    );
}

/// Run `job` off the GTK thread and hand back its result, error or not.
pub fn attempt<T: Send + 'static>(
    job: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    done: impl FnOnce(anyhow::Result<T>) + 'static,
) {
    nyx_core::cmd::background(
        move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "The request stopped unexpectedly. Try again."
                ))
            })
        },
        done,
    );
}

/// Wait for a background-service job to finish, then hand over its record.
pub fn track(ui: &Rc<Ui>, id: String, done: impl FnOnce(&Rc<Ui>, serde_json::Value) + 'static) {
    crate::status::poll_soon(ui);
    work(ui, move || backup_service::wait_job(&id), done);
}

/// A toast at the bottom of the window; a new one replaces any showing.
pub fn toast(ui: &Rc<Ui>, message: &str) {
    if let Some(old) = ui.toast.borrow_mut().take() {
        if old.parent().is_some() {
            ui.overlay.remove_overlay(&old);
        }
    }
    let bx = w::hbox(14);
    bx.add_css_class("toast");
    let l = gtk::Label::new(Some(message));
    l.set_wrap(true);
    l.set_max_width_chars(60);
    bx.append(&l);
    bx.set_halign(gtk::Align::Center);
    bx.set_valign(gtk::Align::End);
    ui.overlay.add_overlay(&bx);
    *ui.toast.borrow_mut() = Some(bx.clone().upcast());
    let overlay = ui.overlay.clone();
    let ms = if message.len() > 80 { 6000 } else { 3500 };
    glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
        if bx.parent().is_some() {
            overlay.remove_overlay(&bx);
        }
    });
}

/// A small card explaining what went wrong.
pub fn error(_ui: &Rc<Ui>, title: &str, body: &str) {
    let sheet = crate::forms::Sheet::new(title, 440);
    let text = label(body, "");
    text.set_selectable(true);
    sheet.body.append(&text);
    sheet.close_button("Close");
    sheet.present();
}

/// A card with longer read-only text (inventory, raw results).
pub fn info(title: &str, body: &str) {
    let sheet = crate::forms::Sheet::new(title, 560);
    let text = label(body, "");
    text.set_selectable(true);
    sheet.body.append(&text);
    sheet.close_button("Close");
    sheet.present();
}

pub type MenuItem = (&'static str, bool, Box<dyn Fn()>);

/// A `⋯` button whose popover lists `items` (label, danger, action).
pub fn menu(tip: &str, items: Vec<MenuItem>) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_icon_name("view-more-symbolic");
    button.add_css_class("bar-button");
    button.set_tooltip_text(Some(tip));
    button.set_valign(gtk::Align::Center);
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    let list = w::vbox(2);
    for (text, danger, act) in items {
        if text == "-" {
            list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            continue;
        }
        let b = gtk::Button::with_label(text);
        b.add_css_class("flat");
        b.add_css_class("menu-item");
        if danger {
            b.add_css_class("danger-item");
        }
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        let p = popover.clone();
        b.connect_clicked(move |_| {
            p.popdown();
            act();
        });
        list.append(&b);
    }
    popover.set_child(Some(&list));
    button.set_popover(Some(&popover));
    button
}

/// A readable one-line description of a destination.
pub fn destination_summary(d: &Destination) -> String {
    match &d.kind {
        DestinationKind::Local { path } => format!("Folder · {}", nyx_core::paths::pretty(path)),
        DestinationKind::Mounted { path, .. } => {
            format!("External drive · {}", nyx_core::paths::pretty(path))
        }
        DestinationKind::Nfs { source, .. } => format!("NFS · {source}"),
        DestinationKind::Sftp { host, path } => format!("SFTP · {host}:{path}"),
        DestinationKind::Smb { server, share, .. } => format!("SMB · {server}/{share}"),
        DestinationKind::S3 {
            bucket, endpoint, ..
        } => {
            let host = endpoint
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/');
            if host.is_empty() {
                format!("S3 · {bucket}")
            } else {
                format!("S3 · {bucket} on {host}")
            }
        }
    }
}

/// An ID for a new profile or destination, from its name, unused in `taken`.
pub fn slug(name: &str, taken: &[String]) -> String {
    let mut base: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while base.contains("--") {
        base = base.replace("--", "-");
    }
    let mut base = base.trim_matches('-').to_string();
    base.truncate(48);
    let base = if base.is_empty() {
        "backup".to_string()
    } else {
        base
    };
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slugs_are_valid_ids() {
        let taken = vec!["personal-data".to_string()];
        assert_eq!(slug("Personal data", &taken), "personal-data-2");
        assert_eq!(slug("  ~~ ", &[]), "backup");
        assert!(valid_id(&slug("--Weird  Name!!", &[])).is_ok());
    }
    #[test]
    fn counts_group_digits() {
        assert_eq!(count(3412, "file", "files"), "3,412 files");
        assert_eq!(count(1, "file", "files"), "1 file");
    }
}
