//! Opt-in fixture renderer. Not compiled into installed builds and never drives the desktop.
use super::*;
use std::time::Duration;
fn descendants(widget: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
    out.push(widget.clone());
    let mut child = widget.first_child();
    while let Some(c) = child {
        descendants(&c, out);
        child = c.next_sibling();
    }
}
fn click(root: &gtk::Widget, tooltip: &str) -> bool {
    let mut all = Vec::new();
    descendants(root, &mut all);
    for w in all {
        if let Ok(b) = w.downcast::<gtk::Button>() {
            let mut content = Vec::new();
            descendants(b.upcast_ref(), &mut content);
            if b.tooltip_text().is_some_and(|s| s == tooltip)
                || b.label().is_some_and(|s| s == tooltip)
                || content
                    .iter()
                    .filter_map(|w| w.clone().downcast::<gtk::Label>().ok())
                    .any(|l| l.text() == tooltip)
            {
                b.emit_clicked();
                return true;
            }
        }
    }
    false
}
fn render(widget: &gtk::Widget, path: &Path, width: i32, height: i32) -> anyhow::Result<()> {
    widget.allocate(width, height, -1, None);
    let content = gtk::Snapshot::new();
    if let Some(parent) = widget.parent() {
        parent.snapshot_child(widget, &content);
    } else {
        let mut child = widget.first_child();
        while let Some(c) = child {
            widget.snapshot_child(&c, &content);
            child = c.next_sibling();
        }
    }
    let node = content
        .to_node()
        .ok_or_else(|| anyhow::anyhow!("Widget content was not rendered"))?;
    let snapshot = gtk::Snapshot::new();
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
fn dialog_window(ui: &Ui) -> gtk::Dialog {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Dialog>().ok())
        .find(|w| w.is_visible() && w.transient_for().is_some_and(|p| p == ui.window))
        .expect("Expected visible fixture dialog")
}
fn field(root: &gtk::Widget, name: &str) -> gtk::Entry {
    let mut widgets = Vec::new();
    descendants(root, &mut widgets);
    widgets
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Entry>().ok())
        .find(|e| {
            e.parent()
                .and_then(|p| p.first_child())
                .and_then(|w| w.downcast::<gtk::Label>().ok())
                .is_some_and(|l| l.text() == name)
        })
        .expect("Expected form field")
}
fn native_render(w: &gtk::Window, path: &Path) {
    render(w.upcast_ref(), path, w.width(), w.height()).unwrap();
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
pub fn run(ui: &Rc<Ui>) {
    let Some(folder) = std::env::var_os("NYX_BACKUP_UI_REVIEW") else {
        return;
    };
    let folder = PathBuf::from(folder);
    std::fs::create_dir_all(&folder).unwrap();
    let step = Rc::new(Cell::new(0));
    let u = ui.clone();
    let began = std::time::Instant::now();
    glib::timeout_add_local(Duration::from_millis(300), move || {
        if began.elapsed() < Duration::from_secs(3) {
            return glib::ControlFlow::Continue;
        }
        let i = step.get();
        let page = ["overview", "backups", "restore", "settings"];
        if i < 16 {
            let n = i / 2;
            let width = if n < 4 { 1600 } else { 800 };
            if i % 2 == 0 {
                if n % 4 == 3 {
                    settings(&u);
                } else {
                    if settings_card::is_open() {
                        settings_card::escape();
                    }
                    u.stack.set_visible_child_name(page[n % 4]);
                }
                u.window.set_default_size(width, 900);
                if let Some(root) = u.window.child() {
                    root.allocate(width, 900, -1, None);
                }
            } else if let Some(root) = u.window.child() {
                render(
                    &root,
                    &folder.join(format!("{}-{width}.png", page[n % 4])),
                    width,
                    900,
                )
                .unwrap();
            }
            step.set(i + 1);
            return glib::ControlFlow::Continue;
        }
        if i == 16 {
            if !settings_card::is_open() {
                settings(&u);
            }
            let hit = click(u.window.upcast_ref(), "New destination");
            assert!(hit, "Destination button missing");
            step.set(17);
            return glib::ControlFlow::Continue;
        }
        if i == 17 {
            let d = dialog_window(&u);
            native_render(d.upcast_ref(), &folder.join("destination-local.png"));
            let mut all = Vec::new();
            descendants(d.upcast_ref(), &mut all);
            all.into_iter()
                .find_map(|w| w.downcast::<gtk::DropDown>().ok())
                .expect("Destination selector")
                .set_selected(5);
            step.set(18);
            return glib::ControlFlow::Continue;
        }
        if i == 18 {
            let d = dialog_window(&u);
            native_render(d.upcast_ref(), &folder.join("destination-s3.png"));
            let mut all = Vec::new();
            descendants(d.upcast_ref(), &mut all);
            let selector = all
                .into_iter()
                .find_map(|w| w.downcast::<gtk::DropDown>().ok())
                .unwrap();
            for transport in 0..=5 {
                selector.set_selected(transport);
                for (name, visible) in [
                    ("S3 region", transport == 5),
                    ("SMB username", transport == 4),
                    ("NFS source (server:/export)", transport == 2),
                    (
                        "SSH host alias or SMB server",
                        transport == 3 || transport == 4,
                    ),
                ] {
                    assert_eq!(
                        field(d.upcast_ref(), name).parent().unwrap().is_visible(),
                        visible,
                        "Wrong transport form fields"
                    );
                }
            }
            selector.set_selected(0);
            let count = Store::default().load().unwrap().destinations.len();
            d.response(gtk::ResponseType::Accept);
            assert!(d.is_visible(), "Invalid destination save closed the editor");
            assert_eq!(
                Store::default().load().unwrap().destinations.len(),
                count,
                "Invalid save mutated configuration"
            );
            step.set(19);
            return glib::ControlFlow::Continue;
        }
        if i == 19 {
            let d = dialog_window(&u);
            native_render(d.upcast_ref(), &folder.join("destination-validation.png"));
            d.response(gtk::ResponseType::Cancel);
            assert!(!d.is_visible(), "Cancel did not dismiss destination editor");
            assert!(click(
                u.window.upcast_ref(),
                "Cloud, GitHub sync and integration grants"
            ));
            step.set(20);
            return glib::ControlFlow::Continue;
        }
        if i == 20 {
            let d = dialog_window(&u);
            native_render(d.upcast_ref(), &folder.join("ecosystem-dialog.png"));
            assert!(key(
                &d,
                gtk::gdk::Key::Escape,
                gtk::gdk::ModifierType::empty()
            ));
            assert!(!d.is_visible(), "Escape did not dismiss ecosystem dialog");
            if settings_card::is_open() {
                settings_card::escape();
            }
            u.stack.set_visible_child_name("restore");
            assert!(click(
                u.window.upcast_ref(),
                "Offline disk recovery and boot kit"
            ));
            step.set(21);
            return glib::ControlFlow::Continue;
        }
        if i == 21 {
            let d = dialog_window(&u);
            native_render(d.upcast_ref(), &folder.join("recovery-dialog.png"));
            key(&d, gtk::gdk::Key::Escape, gtk::gdk::ModifierType::empty());
            assert!(!d.is_visible());
            u.stack.set_visible_child_name("backups");
            assert!(click(u.window.upcast_ref(), "New profile"));
            step.set(22);
            return glib::ControlFlow::Continue;
        }
        if i == 22 {
            let d = dialog_window(&u);
            native_render(d.upcast_ref(), &folder.join("profile-dialog.png"));
            d.response(gtk::ResponseType::Cancel);
            assert!(!d.is_visible());
            assert!(key(
                &u.window,
                gtk::gdk::Key::P,
                gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK
            ));
            step.set(23);
            return glib::ControlFlow::Continue;
        }
        if i == 23 {
            let palette = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Window>().ok())
                .find(|w| w.is_visible() && w.title().is_some_and(|s| s == "Go to"))
                .expect("Ctrl+Shift+P did not open the shared palette");
            native_render(&palette, &folder.join("command-palette.png"));
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
            assert!(
                !u.window.is_visible(),
                "Ctrl+Q did not close the fixture window"
            );
            std::fs::write(folder.join("result.json"),r#"{"pages_rendered":8,"transport_fields":6,"invalid_save_guard":true,"destination_cancel":true,"ecosystem_escape":true,"profile_cancel":true,"recovery_escape":true,"command_palette":true,"ctrl_q":true}"#).unwrap();
            app.quit();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Break
    });
}
