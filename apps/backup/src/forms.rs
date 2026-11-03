//! Sheets: small cards over the window for editors, reviews and confirmations,
//! with the safe choice first and errors shown in place.
use crate::ui::*;
use gtk::{glib, prelude::*};
use nyx_ui::widgets as w;
use std::{cell::RefCell, rc::Rc};

thread_local! {
    /// The window sheets open over: its overlay and the window itself.
    static HOST: RefCell<Option<(gtk::Overlay, gtk::Window)>> = const { RefCell::new(None) };
    /// Open sheets, newest last.
    static OPEN: RefCell<Vec<Sheet>> = const { RefCell::new(Vec::new()) };
}

/// Sheets open inside `window`, over `overlay`.
pub fn attach(overlay: &gtk::Overlay, window: &impl IsA<gtk::Window>) {
    HOST.with(|h| *h.borrow_mut() = Some((overlay.clone(), window.clone().upcast())));
}

/// The newest open sheet.
pub fn top() -> Option<Sheet> {
    OPEN.with(|o| o.borrow().last().cloned())
}

/// A card drawn inside the window, over a veil like the Settings card's. Not a
/// window of its own: a separate window would be dimmed by the compositor's
/// modal dimming and decorated as another app's would.
#[derive(Clone)]
pub struct Sheet {
    /// The app's window, for file choosers.
    pub parent: gtk::Window,
    pub(crate) veil: gtk::Box,
    pub body: gtk::Box,
    pub error: gtk::Label,
    pub buttons: gtk::Box,
}

impl PartialEq for Sheet {
    fn eq(&self, other: &Self) -> bool {
        self.veil == other.veil
    }
}

impl Sheet {
    pub fn new(title: &str, width: i32) -> Sheet {
        let parent = HOST
            .with(|h| h.borrow().as_ref().map(|(_, w)| w.clone()))
            .expect("Sheets need the main window");
        let veil = w::vbox(0);
        veil.add_css_class("settings-veil");
        veil.add_css_class("sheet-veil");
        veil.set_hexpand(true);
        veil.set_vexpand(true);
        let card = w::vbox(12);
        card.add_css_class("dialog-card");
        card.add_css_class("backup-sheet");
        card.set_overflow(gtk::Overflow::Hidden);
        card.append(&w::label(title, "section-title"));
        let body = w::vbox(10);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(520)
            .child(&body)
            .build();
        card.append(&scroll);
        let error = label("", "sheet-error");
        error.set_visible(false);
        card.append(&error);
        let footer = w::hbox(8);
        footer.add_css_class("sheet-footer");
        let buttons = w::hbox(8);
        buttons.set_halign(gtk::Align::End);
        buttons.set_hexpand(true);
        footer.append(&buttons);
        card.append(&footer);
        // At most `width` wide, narrower when the window is.
        let clamp = nyx_ui::clamp::Clamp::new(&card, width);
        clamp.set_valign(gtk::Align::Center);
        clamp.set_vexpand(true);
        veil.append(&clamp);
        let sheet = Sheet {
            parent,
            veil,
            body,
            error,
            buttons,
        };
        let keys = gtk::EventControllerKey::new();
        let s = sheet.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                s.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        sheet.veil.add_controller(keys);
        sheet
    }
    pub fn present(&self) {
        let Some(overlay) = HOST.with(|h| h.borrow().as_ref().map(|(o, _)| o.clone())) else {
            return;
        };
        overlay.add_overlay(&self.veil);
        overlay.set_measure_overlay(&self.veil, false);
        OPEN.with(|o| o.borrow_mut().push(self.clone()));
        // Keyboard focus moves into the sheet, off the controls behind it.
        let veil = self.veil.clone();
        glib::idle_add_local_once(move || {
            veil.child_focus(gtk::DirectionType::TabForward);
        });
    }
    pub fn close(&self) {
        if let Some(overlay) = self.veil.parent().and_downcast::<gtk::Overlay>() {
            overlay.remove_overlay(&self.veil);
        }
        OPEN.with(|o| o.borrow_mut().retain(|s| s != self));
    }
    pub fn fail(&self, message: &str) {
        self.error.set_text(message);
        self.error.set_visible(!message.is_empty());
    }
    /// A plain button that closes the sheet.
    pub fn close_button(&self, text: &str) -> gtk::Button {
        let b = gtk::Button::with_label(text);
        let s = self.clone();
        b.connect_clicked(move |_| s.close());
        self.buttons.append(&b);
        b
    }
    /// The sheet's main action; `primary` makes it the accent button.
    pub fn action(&self, text: &str, primary: bool) -> gtk::Button {
        let b = gtk::Button::with_label(text);
        if primary {
            b.add_css_class("suggested-action");
        }
        self.buttons.append(&b);
        b
    }
    /// A two-click destructive action (the first click arms it for 4 s).
    pub fn danger(&self, text: &str, armed: &str, act: impl Fn() + 'static) -> gtk::Button {
        let b = w::two_click(text, armed, act);
        self.buttons.append(&b);
        b
    }
    /// Run `job` off the GTK thread with the sheet's buttons disabled. The
    /// sheet closes when it succeeds and shows the error in place otherwise,
    /// so nothing the user entered or confirmed is lost.
    pub fn submit<T: Send + 'static>(
        &self,
        ui: &Rc<Ui>,
        job: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(&Rc<Ui>, T) + 'static,
    ) {
        self.fail("");
        self.buttons.set_sensitive(false);
        let sheet = self.clone();
        let ui = ui.clone();
        attempt(job, move |result| {
            sheet.buttons.set_sensitive(true);
            match result {
                Ok(v) => {
                    sheet.close();
                    done(&ui, v);
                }
                Err(e) => sheet.fail(&e.to_string()),
            }
        });
    }
}

/// A form row: the field's name over the control, with an optional hint.
pub fn field(
    body: &gtk::Box,
    title: &str,
    hint: &str,
    control: &impl IsA<gtk::Widget>,
) -> gtk::Box {
    let group = w::vbox(4);
    group.add_css_class("form-field");
    group.append(&line(title, "field-title"));
    group.append(control);
    if !hint.is_empty() {
        group.append(&label(hint, "field-hint"));
    }
    body.append(&group);
    group
}

pub fn entry(body: &gtk::Box, title: &str, hint: &str, value: &str) -> gtk::Entry {
    let e = gtk::Entry::builder().text(value).hexpand(true).build();
    field(body, title, hint, &e);
    e
}

/// A switch row: title and description at the left, the switch at the right.
pub fn switch(body: &gtk::Box, title: &str, desc: &str, on: bool) -> gtk::Switch {
    let s = gtk::Switch::new();
    s.set_active(on);
    s.set_valign(gtk::Align::Center);
    let row = w::row(title, desc, Some(s.upcast_ref()));
    row.add_css_class("form-option");
    body.append(&row);
    s
}

/// A collapsed "Advanced" section.
pub fn advanced(body: &gtk::Box) -> gtk::Box {
    let inner = w::vbox(10);
    inner.set_margin_top(8);
    let ex = gtk::Expander::new(Some("Advanced"));
    ex.add_css_class("form-advanced");
    ex.set_child(Some(&inner));
    body.append(&ex);
    inner
}

pub fn text_lines(v: &gtk::TextView) -> Vec<String> {
    let b = v.buffer();
    b.text(&b.start_iter(), &b.end_iter(), false)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

pub fn text_area(body: &gtk::Box, title: &str, hint: &str, value: &str) -> gtk::TextView {
    let v = gtk::TextView::new();
    v.set_wrap_mode(gtk::WrapMode::WordChar);
    v.add_css_class("form-text");
    v.buffer().set_text(value);
    let sc = gtk::ScrolledWindow::builder()
        .child(&v)
        .min_content_height(72)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    sc.add_css_class("form-text-scroll");
    field(body, title, hint, &sc);
    v
}

/// Ask before something that can't be undone. `act` runs on the second click.
pub fn confirm(title: &str, body: &str, button: &str, act: impl Fn() + 'static) {
    let sheet = Sheet::new(title, 420);
    sheet.body.append(&label(body, ""));
    sheet.close_button("Cancel");
    let b = sheet.action(button, false);
    b.add_css_class("destructive-action");
    let s = sheet.clone();
    b.connect_clicked(move |_| {
        s.close();
        act();
    });
    sheet.present();
}

/// Choose a folder; `done` gets its path.
pub fn choose_folder(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    done: impl Fn(std::path::PathBuf) + 'static,
) {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    dialog.select_folder(Some(parent), gtk::gio::Cancellable::NONE, move |r| {
        if let Ok(f) = r {
            if let Some(p) = f.path() {
                done(p);
            }
        }
    });
}

/// Choose a new file to save; `done` gets its path.
pub fn choose_save(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    name: &str,
    done: impl Fn(std::path::PathBuf) + 'static,
) {
    let dialog = gtk::FileDialog::builder()
        .title(title)
        .initial_name(name)
        .modal(true)
        .build();
    dialog.save(Some(parent), gtk::gio::Cancellable::NONE, move |r| {
        if let Ok(f) = r {
            if let Some(p) = f.path() {
                done(p);
            }
        }
    });
}

/// Keep `entry` from being submitted with nothing in it.
pub fn require(entry: &gtk::Entry, button: &gtk::Button) {
    let b = button.clone();
    let update = move |e: &gtk::Entry| b.set_sensitive(!e.text().trim().is_empty());
    update(entry);
    entry.connect_changed(update);
}

/// After the sheet is shown, focus `widget`.
pub fn focus_later(widget: &impl IsA<gtk::Widget>) {
    let w = widget.clone().upcast::<gtk::Widget>();
    glib::idle_add_local_once(move || {
        w.grab_focus();
    });
}
