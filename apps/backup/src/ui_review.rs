//! Opt-in fixture renderer. Not compiled into installed builds and never drives the desktop.
use crate::ui::Ui;
use backup_core::*;
use gtk::{glib, prelude::*};
use nyx_ui::settings_card;
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

fn descendants(widget: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
    out.push(widget.clone());
    let mut child = widget.first_child();
    while let Some(c) = child {
        descendants(&c, out);
        child = c.next_sibling();
    }
}

fn all(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut v = Vec::new();
    descendants(root.upcast_ref(), &mut v);
    v
}

fn button(root: &impl IsA<gtk::Widget>, text: &str) -> Option<gtk::Button> {
    all(root)
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Button>().ok())
        .find(|b| {
            b.tooltip_text().is_some_and(|s| s == text) || b.label().is_some_and(|s| s == text)
        })
}

fn render(widget: &gtk::Widget, path: &Path, width: i32, height: i32) -> anyhow::Result<()> {
    widget.allocate(width, height, -1, None);
    let content = gtk::Snapshot::new();
    let mut child = widget.first_child();
    while let Some(c) = child {
        widget.snapshot_child(&c, &content);
        child = c.next_sibling();
    }
    let node = content
        .to_node()
        .ok_or_else(|| anyhow::anyhow!("Widget content was not rendered"))?;
    let snapshot = gtk::Snapshot::new();
    #[allow(deprecated)]
    let background = widget
        .style_context()
        .lookup_color("theme_bg")
        .unwrap_or(gtk::gdk::RGBA::BLACK);
    snapshot.append_color(
        &background,
        &gtk::graphene::Rect::new(0., 0., width as f32, height as f32),
    );
    snapshot.append_node(&node);
    let node = snapshot.to_node().unwrap();
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer.realize(None::<&gtk::gdk::Surface>)?;
    let texture = renderer.render_texture(
        &node,
        Some(&gtk::graphene::Rect::new(
            0.,
            0.,
            width as f32,
            height as f32,
        )),
    );
    texture.save_to_png(path)?;
    renderer.unrealize();
    Ok(())
}

/// The open sheet over the main window.
fn sheet(_ui: &Ui) -> Option<gtk::Box> {
    crate::forms::top().map(|s| s.veil.clone())
}

/// Render the whole window, with the sheet over it.
fn window_render(ui: &Ui, path: &Path) {
    if let Some(root) = ui.window.child() {
        let (w, h) = (ui.window.width().max(760), ui.window.height().max(560));
        root.allocate(w, h, -1, None);
        render(&root, path, w, h).unwrap();
    }
}

/// The form field titled `name`, by its label.
fn field(root: &impl IsA<gtk::Widget>, name: &str) -> Option<gtk::Widget> {
    all(root).into_iter().find(|w| {
        w.has_css_class("form-field")
            && w.first_child()
                .and_downcast::<gtk::Label>()
                .is_some_and(|l| l.text() == name)
    })
}

fn key(root: &impl IsA<gtk::Widget>, key: gtk::gdk::Key, mods: gtk::gdk::ModifierType) -> bool {
    let controllers = root.observe_controllers();
    for n in 0..controllers.n_items() {
        if let Some(c) = controllers
            .item(n)
            .and_then(|o| o.downcast::<gtk::EventControllerKey>().ok())
        {
            let handled: bool = c.emit_by_name("key-pressed", &[&key, &0u32, &mods]);
            if handled {
                return true;
            }
        }
    }
    false
}

/// Open one scene in a real window for screenshots (`NYX_BACKUP_UI_SCENE`).
fn scene(ui: &Rc<Ui>, name: &str) {
    let u = ui.clone();
    let name = name.to_string();
    glib::timeout_add_local_once(Duration::from_millis(1200), move || match name.as_str() {
        "browse" => crate::restore_ui::review_open_first(&u),
        "settings" => crate::settings::open(&u, None),
        "integrations" => crate::settings::open(&u, Some("Integrations")),
        "profile" => {
            let p = Store::default().load().unwrap().profiles[0].clone();
            crate::backups::edit(&u, Some(p));
        }
        "destination" => crate::destinations::edit(&u, None),
        _ => {}
    });
}

pub fn run(ui: &Rc<Ui>) {
    if let Ok(name) = std::env::var("NYX_BACKUP_UI_SCENE") {
        scene(ui, &name);
    }
    let Some(folder) = std::env::var_os("NYX_BACKUP_UI_REVIEW") else {
        return;
    };
    let folder = PathBuf::from(folder);
    std::fs::create_dir_all(&folder).unwrap();
    let step = Rc::new(Cell::new(0));
    let u = ui.clone();
    let began = std::time::Instant::now();
    let pages = ["backups", "restore", "activity", "settings"];
    glib::timeout_add_local(Duration::from_millis(300), move || {
        if began.elapsed() < Duration::from_secs(3) {
            return glib::ControlFlow::Continue;
        }
        let i = step.get();
        step.set(i + 1);
        if i < 16 {
            let n = i / 2;
            let (width, height) = if n < 4 { (760, 560) } else { (1280, 800) };
            let page = pages[n % 4];
            if i % 2 == 0 {
                if page == "settings" {
                    crate::settings::open(&u, None);
                } else {
                    if settings_card::is_open() {
                        settings_card::close();
                    }
                    u.show(page);
                }
                u.window.set_default_size(width, height);
                settings_card::fit(&u.window);
                if let Some(root) = u.window.child() {
                    root.allocate(width, height, -1, None);
                }
            } else if let Some(root) = u.window.child() {
                render(
                    &root,
                    &folder.join(format!("{page}-{width}.png")),
                    width,
                    height,
                )
                .unwrap();
            }
            return glib::ControlFlow::Continue;
        }
        match i {
            16 => {
                settings_card::close();
                u.show("backups");
                crate::destinations::edit(&u, None);
            }
            17 => {
                let d = sheet(&u).expect("Destination sheet");
                window_render(&u, &folder.join("destination-sheet.png"));
                let selector = all(&d)
                    .into_iter()
                    .find_map(|w| w.downcast::<gtk::DropDown>().ok())
                    .expect("Destination type");
                for transport in 0..=5u32 {
                    selector.set_selected(transport);
                    for (name, visible) in [
                        ("Region", transport == 5),
                        ("Username", transport == 4),
                        ("Export", transport == 2),
                        ("Share", transport == 4),
                    ] {
                        assert_eq!(
                            field(&d, name).expect("Field").is_visible(),
                            visible,
                            "Wrong fields for destination type {transport}"
                        );
                    }
                }
                selector.set_selected(5);
                window_render(&u, &folder.join("destination-s3.png"));
                selector.set_selected(0);
                button(&d, "Add").expect("Add button").emit_clicked();
            }
            18 => {}
            19 => {
                let d = sheet(&u).expect("An invalid destination closed its sheet");
                assert_eq!(
                    Store::default().load().unwrap().destinations.len(),
                    1,
                    "Invalid save changed the configuration"
                );
                window_render(&u, &folder.join("destination-validation.png"));
                assert!(key(
                    &d,
                    gtk::gdk::Key::Escape,
                    gtk::gdk::ModifierType::empty()
                ));
                assert!(d.parent().is_none(), "Esc didn't close the sheet");
                let p = Store::default().load().unwrap().profiles[0].clone();
                crate::backups::edit(&u, Some(p));
            }
            20 => {
                let d = sheet(&u).expect("Profile sheet");
                window_render(&u, &folder.join("profile-sheet.png"));
                button(&d, "Cancel").expect("Cancel").emit_clicked();
                crate::restore_ui::open_profile(&u, "fixture");
            }
            21 => {
                assert!(key(
                    &u.window,
                    gtk::gdk::Key::P,
                    gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK
                ));
            }
            22 => {
                let palette = gtk::Window::list_toplevels()
                    .into_iter()
                    .filter_map(|w| w.downcast::<gtk::Window>().ok())
                    .find(|w| w.is_visible() && w.title().is_some_and(|s| s == "Go to"))
                    .expect("Ctrl+Shift+P did not open the shared palette");
                let (w, h) = (palette.width().max(420), palette.height().max(200));
                render(
                    palette.upcast_ref(),
                    &folder.join("command-palette.png"),
                    w,
                    h,
                )
                .unwrap();
                key(
                    &palette,
                    gtk::gdk::Key::Escape,
                    gtk::gdk::ModifierType::empty(),
                );
                assert!(!palette.is_visible());
                let app = u.window.application().unwrap();
                assert!(key(
                    &u.window,
                    gtk::gdk::Key::q,
                    gtk::gdk::ModifierType::CONTROL_MASK
                ));
                assert!(!u.window.is_visible(), "Ctrl+Q did not close the window");
                std::fs::write(
                    folder.join("result.json"),
                    r#"{"pages_rendered":8,"transport_fields":6,"invalid_save_guard":true,"sheet_escape":true,"profile_cancel":true,"command_palette":true,"ctrl_q":true}"#,
                )
                .unwrap();
                app.quit();
                return glib::ControlFlow::Break;
            }
            _ => return glib::ControlFlow::Break,
        }
        glib::ControlFlow::Continue
    });
}
