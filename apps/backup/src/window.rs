//! The window: a compact header with the view switch, the three views, and
//! the keys every Nyx app answers.
use crate::ui::*;
use backup_core::*;
use gtk::{gdk, glib, prelude::*};
use nyx_ui::{chrome, settings_card, widgets as w};
use std::{cell::RefCell, rc::Rc};

pub fn build(app: &gtk::Application) -> Rc<Ui> {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Backup")
        .default_width(760)
        .default_height(560)
        .build();
    window.add_css_class("backup-window");
    window.set_size_request(640, 440);
    let overlay = gtk::Overlay::new();
    let outer = w::vbox(0);
    outer.add_css_class("window-frame");
    overlay.set_child(Some(&outer));
    window.set_child(Some(&overlay));
    crate::forms::attach(&overlay, &window);

    let header = chrome::app_header(&window, APP_ID, "Backup");
    let views = gtk::Stack::new();
    views.set_vexpand(true);
    views.set_transition_type(gtk::StackTransitionType::Crossfade);
    views.set_transition_duration(if nyx_theme::app_prefs().reduce_motion {
        0
    } else {
        120
    });
    let actions = gtk::Stack::new();
    actions.set_hhomogeneous(false);
    let switch = {
        let (views, actions) = (views.clone(), actions.clone());
        w::segmented(&w::opts(VIEWS), "backups", move |id| {
            views.set_visible_child_name(&id);
            actions.set_visible_child_name(&id);
        })
    };
    switch.add_css_class("view-switch");
    header.center.append(&switch);
    header.end.append(&actions);
    outer.append(&header.root);
    outer.append(&views);

    let backups = crate::backups::View::new();
    let restore = crate::restore_ui::View::new();
    let activity = crate::activity::View::new();
    views.add_named(&backups.root, Some("backups"));
    views.add_named(&restore.root, Some("restore"));
    views.add_named(&activity.root, Some("activity"));

    let ui = Rc::new(Ui {
        window: window.clone(),
        overlay,
        header,
        views: views.clone(),
        switch,
        actions: actions.clone(),
        backups,
        restore,
        activity,
        settings: crate::settings::Page::new(),
        health: RefCell::new(None),
        toast: RefCell::new(None),
    });

    let u = ui.clone();
    let new = w::icon_button("list-add-symbolic", "New backup (Ctrl+N)");
    new.connect_clicked(move |_| crate::backups::edit(&u, None));
    actions.add_named(&new, Some("backups"));
    let u = ui.clone();
    actions.add_named(
        &icon_button("view-refresh-symbolic", "Refresh (Ctrl+R)", move || {
            crate::restore_ui::refresh(&u);
            crate::restore_ui::shown(&u);
        }),
        Some("restore"),
    );
    let u = ui.clone();
    actions.add_named(
        &icon_button("view-refresh-symbolic", "Refresh (Ctrl+R)", move || {
            crate::activity::refresh(&u)
        }),
        Some("activity"),
    );
    let u = ui.clone();
    ui.header.end.append(&chrome::settings_button(move || {
        crate::settings::open(&u, None)
    }));

    let u = ui.clone();
    views.connect_visible_child_name_notify(move |s| {
        let name = s.visible_child_name().unwrap_or_default();
        let title = VIEWS
            .iter()
            .find(|(id, _)| *id == name.as_str())
            .map(|(_, t)| *t)
            .unwrap_or("Backups");
        chrome::set_title(&u.window, "Backup", title);
        let _ = atomic(&last_view_file(), name.as_bytes());
        match name.as_str() {
            "restore" => crate::restore_ui::shown(&u),
            "activity" => crate::activity::refresh(&u),
            _ => {}
        }
    });
    chrome::set_title(&window, "Backup", "Backups");

    crate::restore_ui::setup(&ui);
    keys(&ui);
    crate::backups::refresh(&ui);
    crate::restore_ui::refresh(&ui);
    crate::status::start(&ui);
    let last = std::fs::read_to_string(last_view_file()).unwrap_or_default();
    let start = nyx_theme::start_page("backups", last.trim());
    if VIEWS.iter().any(|(id, _)| *id == start) {
        ui.show(&start);
    }
    ui
}

/// Where the view shown last is kept, for "Open on: the last page used".
fn last_view_file() -> std::path::PathBuf {
    Store::default().state.join("last-view")
}

fn keys(ui: &Rc<Ui>) {
    nyx_ui::keys::install(
        &ui.window,
        nyx_ui::keys::Common {
            close: true,
            ..Default::default()
        },
    );
    let keys = gtk::EventControllerKey::new();
    let u = ui.clone();
    keys.connect_key_pressed(move |_, k, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        match k {
            gdk::Key::Escape if crate::forms::top().is_some() => {
                if let Some(s) = crate::forms::top() {
                    s.close();
                }
            }
            gdk::Key::Escape if settings_card::is_open() => settings_card::escape(),
            gdk::Key::F1 => crate::settings::open(&u, Some("Keyboard")),
            gdk::Key::comma if ctrl => crate::settings::open(&u, None),
            gdk::Key::k | gdk::Key::K if ctrl => nyx_ui::palette::open(&u.window),
            gdk::Key::_1 if ctrl => u.show("backups"),
            gdk::Key::_2 if ctrl => u.show("restore"),
            gdk::Key::_3 if ctrl => u.show("activity"),
            gdk::Key::n | gdk::Key::N if ctrl => crate::backups::edit(&u, None),
            gdk::Key::r | gdk::Key::R if ctrl => match u.view().as_str() {
                "restore" => crate::restore_ui::shown(&u),
                "activity" => crate::activity::refresh(&u),
                _ => {
                    crate::backups::refresh(&u);
                    crate::status::poll_soon(&u);
                }
            },
            gdk::Key::Return | gdk::Key::KP_Enter if ctrl => {
                if let Some(p) = crate::backups::focused_or_first(&u) {
                    crate::backups::back_up(&u, &p);
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    ui.window.add_controller(keys);
}
