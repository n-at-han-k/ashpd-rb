//! The portals that are a conversation rather than a question.
//!
//! Each of these is created, configured, started, and then lives until it is
//! closed - so each is a class, and the object has to be kept for as long as
//! what it granted is wanted. Let it go and the grant goes with it.
//!
//! They all follow the shape {crate::screencast} sets out, and for the same
//! reason: every zbus object is created, used and dropped inside one
//! `block_on`, because one dropped outside the runtime panics, and a panic
//! across this boundary takes the VM rather than raising.
//!
//!   - org.freedesktop.portal.Location
//!   - org.freedesktop.portal.GlobalShortcuts
//!   - org.freedesktop.portal.RemoteDesktop

use std::os::fd::IntoRawFd;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::StreamExt;
use magnus::{function, method, prelude::*, Error, RArray, RHash, RModule, Ruby, TryConvert};

use ashpd_core::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use ashpd_core::desktop::location::{Accuracy, CreateSessionOptions as LocationOptions, LocationProxy};
use ashpd_core::desktop::remote_desktop::{
    DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions,
};
use ashpd_core::desktop::{PersistMode, Session};
use ashpd_core::enumflags2::BitFlags;

use crate::{block, option_bool, option_name, option_number, rt_err, runtime};

/// How long a wait for an event will sit there before giving up, when the
/// caller named no limit. A wait without a bound is a hung Ruby thread.
const DEFAULT_TIMEOUT: f64 = 30.0;

fn timeout_of(seconds: Option<f64>) -> Duration {
    Duration::from_secs_f64(seconds.unwrap_or(DEFAULT_TIMEOUT).max(0.0))
}

// --- org.freedesktop.portal.Location ---------------------------------------

/// Where the user is, if they allow it.
///
/// The portal does not answer with a position: it starts sending them, and
/// {next_location} waits for the next one. The first can take a while - the
/// host may have to turn a radio on - so the timeout matters.
#[magnus::wrap(class = "Ashpd::Location::Session", free_immediately, size)]
struct LocationSession {
    parts: Mutex<Option<(LocationProxy, Session<LocationProxy>)>>,
}

fn accuracy_from(name: &str) -> Result<Accuracy, Error> {
    match name {
        "none" => Ok(Accuracy::None),
        "country" => Ok(Accuracy::Country),
        "city" => Ok(Accuracy::City),
        "neighborhood" => Ok(Accuracy::Neighborhood),
        "street" => Ok(Accuracy::Street),
        "exact" => Ok(Accuracy::Exact),
        other => Err(rt_err(format!(
            "unknown accuracy {other:?}, wanted :none, :country, :city, :neighborhood, :street or :exact"
        ))),
    }
}

impl LocationSession {
    /// `accuracy:` is what to ask the host for, and asking for less than
    /// `:exact` is the polite default when a city will do.
    fn new(ruby: &Ruby, options: RHash) -> Result<Self, Error> {
        let mut create = LocationOptions::default();

        if let Some(name) = option_name(ruby, options, "accuracy")? {
            create = create.set_accuracy(accuracy_from(&name)?);
        }
        if let Some(distance) = option_number(ruby, options, "distance_threshold")? {
            create = create.set_distance_threshold(distance as u32);
        }
        if let Some(time) = option_number(ruby, options, "time_threshold")? {
            create = create.set_time_threshold(time as u32);
        }

        let parts = block(async {
            let proxy = LocationProxy::new().await?;
            let session = proxy.create_session(create).await?;

            Ok::<_, ashpd_core::Error>((proxy, session))
        })?
        .map_err(rt_err)?;

        Ok(Self {
            parts: Mutex::new(Some(parts)),
        })
    }

    /// Start locating, which is where the user is asked.
    fn start(&self) -> Result<(), Error> {
        let guard = self.parts.lock().map_err(rt_err)?;
        let (proxy, session) = guard
            .as_ref()
            .ok_or_else(|| rt_err("the location session is closed"))?;

        block(async { proxy.start(session, None, Default::default()).await?.response() })?.map_err(rt_err)?;

        Ok(())
    }

    /// Wait for the next position, as `{"latitude" =>, "longitude" =>,
    /// "altitude" =>, "accuracy" =>, "speed" =>, "heading" =>,
    /// "description" =>}`, or nil if none arrived in time.
    ///
    /// The first one can be slow - the host may have to turn a radio on - so
    /// the timeout is worth setting generously.
    fn next_location(ruby: &Ruby, rb_self: &Self, seconds: Option<f64>) -> Result<Option<RHash>, Error> {
        let guard = rb_self.parts.lock().map_err(rt_err)?;
        let (proxy, _session) = guard
            .as_ref()
            .ok_or_else(|| rt_err("the location session is closed"))?;

        let located = block(async {
            let mut stream = proxy.receive_location_updated().await?;

            Ok::<_, ashpd_core::Error>(
                tokio::time::timeout(timeout_of(seconds), stream.next())
                    .await
                    .ok()
                    .flatten(),
            )
        })?
        .map_err(rt_err)?;

        let Some(location) = located else {
            return Ok(None);
        };

        let hash = ruby.hash_new();
        hash.aset("latitude", location.latitude())?;
        hash.aset("longitude", location.longitude())?;
        hash.aset("altitude", location.altitude())?;
        hash.aset("accuracy", location.accuracy())?;
        hash.aset("speed", location.speed())?;
        hash.aset("heading", location.heading())?;
        hash.aset("description", location.description())?;

        Ok(Some(hash))
    }

    fn close(&self) -> Result<(), Error> {
        let Some((_proxy, session)) = self.parts.lock().map_err(rt_err)?.take() else {
            return Ok(());
        };

        block(async {
            let result = session.close().await;
            drop(session);
            result
        })?
        .map_err(rt_err)
    }
}

impl Drop for LocationSession {
    fn drop(&mut self) {
        let Ok(mut guard) = self.parts.lock() else { return };
        let Some(parts) = guard.take() else { return };
        let Ok(runtime) = runtime() else { return };

        runtime.block_on(async move { drop(parts) });
    }
}

// --- org.freedesktop.portal.GlobalShortcuts --------------------------------

/// Shortcuts that work whether or not this application has focus.
///
/// The user is shown what is being asked for and may change the keys, so what
/// {bind} returns is what they agreed to, not what was requested.
#[magnus::wrap(class = "Ashpd::GlobalShortcuts::Session", free_immediately, size)]
struct ShortcutsSession {
    parts: Mutex<Option<(GlobalShortcuts, Session<GlobalShortcuts>)>>,
}

impl ShortcutsSession {
    fn new() -> Result<Self, Error> {
        let parts = block(async {
            let proxy = GlobalShortcuts::new().await?;
            let session = proxy.create_session(Default::default()).await?;

            Ok::<_, ashpd_core::Error>((proxy, session))
        })?
        .map_err(rt_err)?;

        Ok(Self {
            parts: Mutex::new(Some(parts)),
        })
    }

    /// Ask for shortcuts, as `[{id:, description:, preferred_trigger:}]`.
    ///
    /// Returns what the user agreed to, as
    /// `[{"id" =>, "description" =>, "trigger_description" =>}]`.
    fn bind(ruby: &Ruby, rb_self: &Self, wanted: RArray) -> Result<RArray, Error> {
        let mut shortcuts = Vec::new();

        for item in wanted.into_iter() {
            let hash = RHash::try_convert(item)?;
            let id = option_name(ruby, hash, "id")?
                .ok_or_else(|| rt_err("every shortcut needs an :id"))?;
            let description = option_name(ruby, hash, "description")?
                .ok_or_else(|| rt_err("every shortcut needs a :description"))?;
            let mut shortcut = NewShortcut::new(id, description);

            if let Some(trigger) = option_name(ruby, hash, "preferred_trigger")? {
                shortcut = shortcut.preferred_trigger(Some(trigger.as_str()));
            }

            shortcuts.push(shortcut);
        }

        let guard = rb_self.parts.lock().map_err(rt_err)?;
        let (proxy, session) = guard
            .as_ref()
            .ok_or_else(|| rt_err("the global shortcuts session is closed"))?;

        let bound = block(async {
            proxy
                .bind_shortcuts(session, &shortcuts, None, Default::default())
                .await?
                .response()
        })?
        .map_err(rt_err)?;

        let array = ruby.ary_new();
        for shortcut in bound.shortcuts() {
            let hash = ruby.hash_new();
            hash.aset("id", shortcut.id())?;
            hash.aset("description", shortcut.description())?;
            hash.aset("trigger_description", shortcut.trigger_description())?;
            let _ = array.push(hash);
        }

        Ok(array)
    }

    /// What is bound now, in the same shape as {bind} returns.
    fn list(ruby: &Ruby, rb_self: &Self) -> Result<RArray, Error> {
        let guard = rb_self.parts.lock().map_err(rt_err)?;
        let (proxy, session) = guard
            .as_ref()
            .ok_or_else(|| rt_err("the global shortcuts session is closed"))?;

        let listed = block(async { proxy.list_shortcuts(session, Default::default()).await?.response() })?
            .map_err(rt_err)?;

        let array = ruby.ary_new();
        for shortcut in listed.shortcuts() {
            let hash = ruby.hash_new();
            hash.aset("id", shortcut.id())?;
            hash.aset("description", shortcut.description())?;
            hash.aset("trigger_description", shortcut.trigger_description())?;
            let _ = array.push(hash);
        }

        Ok(array)
    }

    /// Wait for the user to press one of them, and say which.
    ///
    /// Returns the shortcut's id, or nil if nothing was pressed before the
    /// timeout. Blocks the calling thread, so this belongs on a thread of its
    /// own.
    fn next_activated(rb_self: &Self, seconds: Option<f64>) -> Result<Option<String>, Error> {
        let guard = rb_self.parts.lock().map_err(rt_err)?;
        let (proxy, _session) = guard
            .as_ref()
            .ok_or_else(|| rt_err("the global shortcuts session is closed"))?;

        let activated = block(async {
            let mut stream = proxy.receive_activated().await?;

            Ok::<_, ashpd_core::Error>(
                tokio::time::timeout(timeout_of(seconds), stream.next())
                    .await
                    .ok()
                    .flatten(),
            )
        })?
        .map_err(rt_err)?;

        Ok(activated.map(|event| event.shortcut_id().to_owned()))
    }

    fn close(&self) -> Result<(), Error> {
        let Some((_proxy, session)) = self.parts.lock().map_err(rt_err)?.take() else {
            return Ok(());
        };

        block(async {
            let result = session.close().await;
            drop(session);
            result
        })?
        .map_err(rt_err)
    }
}

impl Drop for ShortcutsSession {
    fn drop(&mut self) {
        let Ok(mut guard) = self.parts.lock() else { return };
        let Some(parts) = guard.take() else { return };
        let Ok(runtime) = runtime() else { return };

        runtime.block_on(async move { drop(parts) });
    }
}

// --- org.freedesktop.portal.RemoteDesktop ----------------------------------

fn device_type_from(name: &str) -> Result<DeviceType, Error> {
    match name {
        "keyboard" => Ok(DeviceType::Keyboard),
        "pointer" => Ok(DeviceType::Pointer),
        "touchscreen" => Ok(DeviceType::Touchscreen),
        other => Err(rt_err(format!(
            "unknown device type {other:?}, wanted :keyboard, :pointer or :touchscreen"
        ))),
    }
}

/// Driving someone's desktop: moving their pointer, pressing their keys.
///
/// The user is asked once, for the devices named in {select_devices}, and the
/// notify methods do nothing until {start} has been answered. Every
/// coordinate is in the stream's own space, not the screen's.
#[magnus::wrap(class = "Ashpd::RemoteDesktop::Session", free_immediately, size)]
struct RemoteDesktopSession {
    parts: Mutex<Option<(RemoteDesktop, Session<RemoteDesktop>)>>,
    started: Mutex<Option<String>>,
}

impl RemoteDesktopSession {
    fn new() -> Result<Self, Error> {
        let parts = block(async {
            let proxy = RemoteDesktop::new().await?;
            let session = proxy.create_session(Default::default()).await?;

            Ok::<_, ashpd_core::Error>((proxy, session))
        })?
        .map_err(rt_err)?;

        Ok(Self {
            parts: Mutex::new(Some(parts)),
            started: Mutex::new(None),
        })
    }

    /// Say which devices to ask for, before starting.
    fn select_devices(ruby: &Ruby, rb_self: &Self, options: RHash) -> Result<(), Error> {
        let mut selection = SelectDevicesOptions::default();

        if let Some(devices) = options.get(ruby.to_symbol("devices")) {
            let mut types = BitFlags::<DeviceType>::empty();

            for device in RArray::try_convert(devices)?.into_iter() {
                types |= device_type_from(&device.to_r_string()?.to_string()?)?;
            }

            selection = selection.set_devices(types);
        }

        if let Some(token) = option_name(ruby, options, "restore_token")? {
            selection = selection.set_restore_token(Some(token.as_str()));
        }

        if option_bool(ruby, options, "persist").unwrap_or(false) {
            selection = selection.set_persist_mode(PersistMode::ExplicitlyRevoked);
        }

        let guard = rb_self.parts.lock().map_err(rt_err)?;
        let (proxy, session) = guard
            .as_ref()
            .ok_or_else(|| rt_err("the remote desktop session is closed"))?;

        block(async { proxy.select_devices(session, selection).await?.response() })?
            .map_err(rt_err)?;

        Ok(())
    }

    /// Start, which is where the user is asked. Returns what they granted, as
    /// `{"devices" =>, "restore_token" =>, "clipboard" =>}`.
    fn start(ruby: &Ruby, rb_self: &Self) -> Result<RHash, Error> {
        let granted = {
            let guard = rb_self.parts.lock().map_err(rt_err)?;
            let (proxy, session) = guard
                .as_ref()
                .ok_or_else(|| rt_err("the remote desktop session is closed"))?;

            block(async { proxy.start(session, None, Default::default()).await?.response() })?.map_err(rt_err)?
        };

        let devices = ruby.ary_new();
        for device_type in granted.devices().iter() {
            let _ = devices.push(ruby.to_symbol(match device_type {
                DeviceType::Keyboard => "keyboard",
                DeviceType::Pointer => "pointer",
                DeviceType::Touchscreen => "touchscreen",
            }));
        }

        if let Some(token) = granted.restore_token() {
            *rb_self.started.lock().map_err(rt_err)? = Some(token.to_owned());
        }

        let hash = ruby.hash_new();
        hash.aset("devices", devices)?;
        hash.aset("restore_token", granted.restore_token())?;
        hash.aset("clipboard", granted.is_clipboard_enabled())?;

        Ok(hash)
    }

    /// The token to keep, so a later session can skip the dialog.
    fn restore_token(&self) -> Result<Option<String>, Error> {
        Ok(self.started.lock().map_err(rt_err)?.clone())
    }

    /// Move the pointer by a relative amount.
    fn move_pointer(&self, dx: f64, dy: f64) -> Result<(), Error> {
        let guard = self.parts.lock().map_err(rt_err)?;
        let (proxy, session) = guard
            .as_ref()
            .ok_or_else(|| rt_err("the remote desktop session is closed"))?;

        block(async { proxy.notify_pointer_motion(session, dx, dy, Default::default()).await })?.map_err(rt_err)
    }

    /// Press or release a pointer button, by its evdev code (BTN_LEFT is
    /// 0x110).
    fn pointer_button(&self, button: i32, pressed: bool) -> Result<(), Error> {
        let guard = self.parts.lock().map_err(rt_err)?;
        let (proxy, session) = guard
            .as_ref()
            .ok_or_else(|| rt_err("the remote desktop session is closed"))?;

        let state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };

        block(async { proxy.notify_pointer_button(session, button, state, Default::default()).await })?
            .map_err(rt_err)
    }

    /// Press or release a key, by its evdev keycode.
    fn key(&self, keycode: i32, pressed: bool) -> Result<(), Error> {
        let guard = self.parts.lock().map_err(rt_err)?;
        let (proxy, session) = guard
            .as_ref()
            .ok_or_else(|| rt_err("the remote desktop session is closed"))?;

        let state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };

        block(async { proxy.notify_keyboard_keycode(session, keycode, state, Default::default()).await })?
            .map_err(rt_err)
    }

    /// A descriptor for an EIS socket, for applications that would rather
    /// speak libei than send one event at a time. Ruby owns it once given.
    fn connect_to_eis(&self) -> Result<i32, Error> {
        let guard = self.parts.lock().map_err(rt_err)?;
        let (proxy, session) = guard
            .as_ref()
            .ok_or_else(|| rt_err("the remote desktop session is closed"))?;

        let fd = block(async { proxy.connect_to_eis(session, Default::default()).await })?.map_err(rt_err)?;

        Ok(fd.into_raw_fd())
    }

    fn close(&self) -> Result<(), Error> {
        let Some((_proxy, session)) = self.parts.lock().map_err(rt_err)?.take() else {
            return Ok(());
        };

        block(async {
            let result = session.close().await;
            drop(session);
            result
        })?
        .map_err(rt_err)
    }
}

impl Drop for RemoteDesktopSession {
    fn drop(&mut self) {
        let Ok(mut guard) = self.parts.lock() else { return };
        let Some(parts) = guard.take() else { return };
        let Ok(runtime) = runtime() else { return };

        runtime.block_on(async move { drop(parts) });
    }
}

pub fn init(ruby: &Ruby, namespace: RModule) -> Result<(), Error> {
    let shortcuts = namespace.define_module("GlobalShortcuts")?;
    let shortcuts_session = shortcuts.define_class("Session", ruby.class_object())?;
    shortcuts_session.define_singleton_method("new", function!(ShortcutsSession::new, 0))?;
    shortcuts_session.define_method("bind", method!(ShortcutsSession::bind, 1))?;
    shortcuts_session.define_method("list", method!(ShortcutsSession::list, 0))?;
    shortcuts_session
        .define_method("next_activated", method!(ShortcutsSession::next_activated, 1))?;
    shortcuts_session.define_method("close", method!(ShortcutsSession::close, 0))?;

    let remote = namespace.define_module("RemoteDesktop")?;
    let remote_session = remote.define_class("Session", ruby.class_object())?;
    remote_session.define_singleton_method("new", function!(RemoteDesktopSession::new, 0))?;
    remote_session
        .define_method("select_devices", method!(RemoteDesktopSession::select_devices, 1))?;
    remote_session.define_method("start", method!(RemoteDesktopSession::start, 0))?;
    remote_session
        .define_method("restore_token", method!(RemoteDesktopSession::restore_token, 0))?;
    remote_session.define_method("move_pointer", method!(RemoteDesktopSession::move_pointer, 2))?;
    remote_session
        .define_method("pointer_button", method!(RemoteDesktopSession::pointer_button, 2))?;
    remote_session.define_method("key", method!(RemoteDesktopSession::key, 2))?;
    remote_session
        .define_method("connect_to_eis", method!(RemoteDesktopSession::connect_to_eis, 0))?;
    remote_session.define_method("close", method!(RemoteDesktopSession::close, 0))?;

    let location = namespace.define_module("Location")?;
    let location_session = location.define_class("Session", ruby.class_object())?;
    location_session.define_singleton_method("new", function!(LocationSession::new, 1))?;
    location_session.define_method("start", method!(LocationSession::start, 0))?;
    location_session
        .define_method("next_location", method!(LocationSession::next_location, 1))?;
    location_session.define_method("close", method!(LocationSession::close, 0))?;

    Ok(())
}
