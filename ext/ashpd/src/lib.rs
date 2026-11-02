//! Ruby bindings for ashpd, the Rust XDG Desktop Portal client, via magnus.
//!
//! The point of this binding is what it does *not* load. libportal reaches the
//! portals through GIO and carries a GLib main loop with it. ashpd speaks the
//! `org.freedesktop.portal.*` D-Bus interfaces itself through zbus, so neither
//! libportal nor libxdp is in the address space -- the same trade oo7-rb makes
//! against libsecret.
//!
//! Each portal is a module function or a class method under `Ashpd`, named for
//! the interface it speaks to: `Ashpd::Screenshot`, `Ashpd::OpenUri`,
//! `Ashpd::Trash`, `Ashpd::Settings`, `Ashpd::Camera`, `Ashpd::Account`.
//!
//! # Blocking
//!
//! Every method on the crate is async. Each binding drives it to completion on
//! a Tokio runtime owned by this extension and returns the result, so the Ruby
//! API is ordinary and synchronous. The calling thread is blocked while it waits
//! and the GVL is held.
//!
//! That matters more here than it does for a keyring. A portal call shows the
//! user a dialog and does not return until they answer it, so a call made on a
//! GTK main loop freezes the window that the dialog is modal to. Put these on a
//! Ruby thread.
//!
//! # Window identifiers
//!
//! A portal dialog wants a parent window to be modal to, which is what
//! `WindowIdentifier` carries. Obtaining one from a live GTK window needs
//! ashpd's `gtk4` feature, and that pulls the gtk4, gdk4x11 and gdk4wayland
//! crates into the build. It is deliberately not enabled yet: every request
//! here is sent without an identifier, so dialogs are parentless. Enabling it
//! is what the XdpGtk4 gap needs.
//!
//! # Provenance
//!
//!   - <https://github.com/bilelmoussaoui/ashpd> (MIT)
//!   - <https://flatpak.github.io/xdg-desktop-portal/docs/> (the interfaces)

use std::fs::File;
use std::future::Future;
use std::sync::OnceLock;

use magnus::{function, prelude::*, Error, RArray, RHash, Ruby};

use ashpd_core::desktop::account::UserInformation;
use ashpd_core::desktop::camera::Camera;
use ashpd_core::desktop::open_uri::OpenFileRequest;
use ashpd_core::desktop::screenshot::Screenshot;
use ashpd_core::desktop::settings::{ColorScheme, Settings};
use ashpd_core::desktop::trash;
use ashpd_core::Uri;
use tokio::runtime::{Builder, Runtime};

// --- helpers ---------------------------------------------------------------

fn rt_err<E: std::fmt::Display>(e: E) -> Error {
    // Safe: every caller runs inside a Ruby method invocation (GVL held).
    let ruby = Ruby::get().expect("rt_err called outside the Ruby VM");
    Error::new(ruby.exception_runtime_error(), e.to_string())
}

/// One runtime for the whole extension, built on first use.
///
/// A current-thread runtime is not enough: zbus spawns its own socket reader
/// task, so the runtime has to keep driving background work while a portal
/// request waits for the user to answer a dialog. Hence the multi-thread
/// flavour with one worker, as in oo7-rb.
fn runtime() -> Result<&'static Runtime, Error> {
    static RUNTIME: OnceLock<std::io::Result<Runtime>> = OnceLock::new();

    RUNTIME
        .get_or_init(|| Builder::new_multi_thread().worker_threads(1).enable_all().build())
        .as_ref()
        .map_err(rt_err)
}

fn block<F: Future>(future: F) -> Result<F::Output, Error> {
    Ok(runtime()?.block_on(future))
}

/// A portal colour, which the interfaces give as three doubles in 0.0..=1.0.
fn color_to(ruby: &Ruby, red: f64, green: f64, blue: f64) -> RArray {
    let array = ruby.ary_new_capa(3);
    let _ = array.push(red);
    let _ = array.push(green);
    let _ = array.push(blue);
    array
}

fn uri_from(uri: &str) -> Result<Uri, Error> {
    Uri::parse(uri).map_err(rt_err)
}

// --- org.freedesktop.portal.Screenshot -------------------------------------

/// Take a screenshot, returning the URI of the file the portal wrote.
///
/// `interactive` lets the user choose the area and is what a user expects from a
/// screenshot tool; without it the portal decides, which on GNOME means the
/// whole screen after its own confirmation.
fn screenshot(interactive: bool, modal: bool) -> Result<String, Error> {
    let request = Screenshot::request().interactive(interactive).modal(modal);
    let response = block(request.send())?.map_err(rt_err)?;

    Ok(response.response().map_err(rt_err)?.uri().as_str().to_owned())
}

/// Pick one colour off the screen, as `[red, green, blue]` in 0.0..=1.0.
fn pick_color(ruby: &Ruby) -> Result<RArray, Error> {
    let proxy = block(ashpd_core::desktop::screenshot::ScreenshotProxy::new())?.map_err(rt_err)?;
    let request = block(proxy.pick_color(None, Default::default()))?.map_err(rt_err)?;
    let color = request.response().map_err(rt_err)?;

    Ok(color_to(ruby, color.red(), color.green(), color.blue()))
}

// --- org.freedesktop.portal.OpenURI ----------------------------------------

/// Hand a URI to whatever the user's desktop opens it with.
///
/// `ask` forces the application chooser even when a default is set.
fn open_uri(uri: String, writeable: bool, ask: bool) -> Result<(), Error> {
    let uri = uri_from(&uri)?;
    let request = OpenFileRequest::default().writeable(writeable).ask(ask);

    block(request.send_uri(&uri))?.map_err(rt_err)?;

    Ok(())
}

// --- org.freedesktop.portal.Trash ------------------------------------------

/// Move a path to the user's trash.
///
/// The interface takes a file descriptor rather than a path, so the file is
/// opened here. It is opened read-only: the portal needs it only to identify
/// what to move, and asking for write would fail on a file the user can trash
/// but not modify.
fn trash_file(path: String) -> Result<(), Error> {
    let file = File::open(&path).map_err(rt_err)?;

    block(trash::trash_file(&file))?.map_err(rt_err)?;

    Ok(())
}

// --- org.freedesktop.portal.Settings ---------------------------------------

/// The user's light/dark preference, as `"no_preference"`, `"prefer_dark"` or
/// `"prefer_light"` -- the interface's own three values, not a boolean, because
/// "no preference" is a distinct state an application is meant to handle.
fn color_scheme() -> Result<String, Error> {
    let settings = block(Settings::new())?.map_err(rt_err)?;
    let scheme = block(settings.color_scheme())?.map_err(rt_err)?;

    Ok(match scheme {
        ColorScheme::NoPreference => "no_preference",
        ColorScheme::PreferDark => "prefer_dark",
        ColorScheme::PreferLight => "prefer_light",
    }
    .to_owned())
}

/// The user's accent colour, as `[red, green, blue]` in 0.0..=1.0.
fn accent_color(ruby: &Ruby) -> Result<RArray, Error> {
    let settings = block(Settings::new())?.map_err(rt_err)?;
    let color = block(settings.accent_color())?.map_err(rt_err)?;

    Ok(color_to(ruby, color.red(), color.green(), color.blue()))
}

// --- org.freedesktop.portal.Camera -----------------------------------------

/// Whether the host has a camera at all.
///
/// This is a property read, not a request, so it shows the user nothing and is
/// the right thing to branch a UI on before asking for access.
fn camera_present() -> Result<bool, Error> {
    let camera = block(Camera::new())?.map_err(rt_err)?;

    block(camera.is_present())?.map_err(rt_err)
}

// --- org.freedesktop.portal.Account ----------------------------------------

/// Ask the user to share their identity, as `{"id" =>, "name" =>, "image" =>}`.
///
/// `reason` is shown to the user in the dialog, so it should say what the
/// application wants it for.
fn user_information(ruby: &Ruby, reason: String) -> Result<RHash, Error> {
    let request = UserInformation::request().reason(reason.as_str());
    let response = block(request.send())?.map_err(rt_err)?;
    let information = response.response().map_err(rt_err)?;

    let hash = ruby.hash_new();
    hash.aset("id", information.id())?;
    hash.aset("name", information.name())?;
    hash.aset("image", information.image().as_str())?;

    Ok(hash)
}

// --- init ------------------------------------------------------------------

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let namespace = ruby.define_module("Ashpd")?;

    let screenshot_module = namespace.define_module("Screenshot")?;
    screenshot_module.define_singleton_method("capture", function!(screenshot, 2))?;
    screenshot_module.define_singleton_method("pick_color", function!(pick_color, 0))?;

    let open_uri_module = namespace.define_module("OpenUri")?;
    open_uri_module.define_singleton_method("open", function!(open_uri, 3))?;

    let trash_module = namespace.define_module("Trash")?;
    trash_module.define_singleton_method("file", function!(trash_file, 1))?;

    let settings_module = namespace.define_module("Settings")?;
    settings_module.define_singleton_method("color_scheme", function!(color_scheme, 0))?;
    settings_module.define_singleton_method("accent_color", function!(accent_color, 0))?;

    let camera_module = namespace.define_module("Camera")?;
    camera_module.define_singleton_method("present?", function!(camera_present, 0))?;

    let account_module = namespace.define_module("Account")?;
    account_module.define_singleton_method("user_information", function!(user_information, 1))?;

    Ok(())
}
