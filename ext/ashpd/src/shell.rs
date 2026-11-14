//! The portals that ask the user for something, or tell the shell something.
//!
//! Every call here may put a dialog in front of the user and does not return
//! until it is answered, so none of them belongs on a main loop - put them on
//! a Ruby thread. The exception is {Background.set_status}, which only updates
//! what the shell says about this application.
//!
//!   - org.freedesktop.portal.Email
//!   - org.freedesktop.portal.Background
//!   - org.freedesktop.portal.Notification
//!   - org.freedesktop.portal.Wallpaper
//!   - org.freedesktop.portal.Inhibit
//!   - org.freedesktop.portal.Secret

use magnus::{function, prelude::*, Error, RArray, RHash, RModule, Ruby, TryConvert};

use ashpd_core::desktop::background::{Background, BackgroundProxy, SetStatusOptions};
use ashpd_core::desktop::email::EmailRequest;
use ashpd_core::desktop::inhibit::{InhibitFlags, InhibitOptions, InhibitProxy};
use ashpd_core::desktop::notification::{Notification, NotificationProxy, Priority};
use ashpd_core::desktop::wallpaper::{SetOn, WallpaperRequest};
use ashpd_core::enumflags2::BitFlags;
use ashpd_core::Uri;

use crate::{block, option_bool, option_name, option_strings, rt_err};

// --- org.freedesktop.portal.Email ------------------------------------------

/// Open the user's mail program on a new message.
///
/// Nothing is sent: the portal hands the draft to whatever composes mail here
/// and the user decides. So the addresses are a suggestion, not an action, and
/// this is the honest way for an application to offer "email us".
fn email_compose(ruby: &Ruby, options: RHash) -> Result<(), Error> {
    let addresses = option_strings(ruby, options, "addresses")?;
    let cc = option_strings(ruby, options, "cc")?;
    let bcc = option_strings(ruby, options, "bcc")?;
    let subject = option_name(ruby, options, "subject")?;
    let body = option_name(ruby, options, "body")?;

    block(async {
        let mut request = EmailRequest::default();

        if !addresses.is_empty() {
            request = request.addresses(&addresses);
        }
        if !cc.is_empty() {
            request = request.cc(&cc);
        }
        if !bcc.is_empty() {
            request = request.bcc(&bcc);
        }
        request = request.subject(subject.as_deref()).body(body.as_deref());

        request.send().await?.response()
    })?
    .map_err(rt_err)?;

    Ok(())
}

// --- org.freedesktop.portal.Background -------------------------------------

/// Ask to keep running once the window is gone, and optionally to be started
/// at login.
///
/// Returns `{"run_in_background" =>, "auto_start" =>}` - what the user
/// granted, which is not necessarily what was asked for.
fn background_request(ruby: &Ruby, options: RHash) -> Result<RHash, Error> {
    let reason = option_name(ruby, options, "reason")?;
    let auto_start = option_bool(ruby, options, "auto_start");
    let dbus_activatable = option_bool(ruby, options, "dbus_activatable");

    let granted: Background = block(async {
        let mut request = ashpd_core::desktop::background::Background::request();

        if let Some(reason) = reason.as_deref() {
            request = request.reason(reason);
        }

        request
            .auto_start(auto_start)
            .dbus_activatable(dbus_activatable)
            .send()
            .await?
            .response()
    })?
    .map_err(rt_err)?;

    let hash = ruby.hash_new();
    hash.aset("run_in_background", granted.run_in_background())?;
    hash.aset("auto_start", granted.auto_start())?;

    Ok(hash)
}

/// Tell the shell what this application is doing while it runs in the
/// background, which is what the user sees when they ask what is still
/// running. This shows no dialog.
fn background_set_status(message: String) -> Result<(), Error> {
    block(async {
        BackgroundProxy::new()
            .await?
            .set_status(SetStatusOptions::default().set_message(&message))
            .await
    })?
    .map_err(rt_err)
}

// --- org.freedesktop.portal.Notification -----------------------------------

fn priority_from(name: &str) -> Result<Priority, Error> {
    match name {
        "low" => Ok(Priority::Low),
        "normal" => Ok(Priority::Normal),
        "high" => Ok(Priority::High),
        "urgent" => Ok(Priority::Urgent),
        other => Err(rt_err(format!(
            "unknown priority {other:?}, wanted :low, :normal, :high or :urgent"
        ))),
    }
}

/// Post a notification under an id of this application's choosing.
///
/// The id is how it is replaced or withdrawn later: posting the same id again
/// replaces what is on screen rather than stacking another one up.
fn notification_add(ruby: &Ruby, id: String, title: String, options: RHash) -> Result<(), Error> {
    let body = option_name(ruby, options, "body")?;
    let priority = option_name(ruby, options, "priority")?;

    block(async {
        let mut notification = Notification::new(&title).body(body.as_deref());

        if let Some(name) = priority.as_deref() {
            notification = notification.priority(priority_from(name).ok());
        }

        NotificationProxy::new()
            .await?
            .add_notification(&id, notification)
            .await
    })?
    .map_err(rt_err)
}

/// Withdraw a notification posted earlier. Unknown ids are not an error: the
/// state asked for - not on screen - is the state that results.
fn notification_remove(id: String) -> Result<(), Error> {
    block(async {
        NotificationProxy::new()
            .await?
            .remove_notification(&id)
            .await
    })?
    .map_err(rt_err)
}

// --- org.freedesktop.portal.Wallpaper --------------------------------------

fn set_on_from(name: &str) -> Result<SetOn, Error> {
    match name {
        "lockscreen" => Ok(SetOn::Lockscreen),
        "background" => Ok(SetOn::Background),
        "both" => Ok(SetOn::Both),
        other => Err(rt_err(format!(
            "unknown wallpaper target {other:?}, wanted :lockscreen, :background or :both"
        ))),
    }
}

/// Set the desktop wallpaper from a URI.
///
/// `show_preview` asks the portal to show the user what it will look like
/// before doing it, which is the difference between offering and imposing.
fn wallpaper_set(ruby: &Ruby, uri: String, options: RHash) -> Result<(), Error> {
    let parsed = Uri::parse(&uri).map_err(rt_err)?;
    let set_on = option_name(ruby, options, "set_on")?;
    let show_preview = option_bool(ruby, options, "show_preview");

    block(async {
        let mut request = WallpaperRequest::default().show_preview(show_preview);

        if let Some(name) = set_on.as_deref() {
            request = request.set_on(set_on_from(name).ok());
        }

        request.build_uri(&parsed).await?.response()
    })?
    .map_err(rt_err)?;

    Ok(())
}

// --- org.freedesktop.portal.Inhibit ----------------------------------------

fn inhibit_flag_from(name: &str) -> Result<InhibitFlags, Error> {
    match name {
        "logout" => Ok(InhibitFlags::Logout),
        "user_switch" => Ok(InhibitFlags::UserSwitch),
        "suspend" => Ok(InhibitFlags::Suspend),
        "idle" => Ok(InhibitFlags::Idle),
        other => Err(rt_err(format!(
            "unknown inhibit flag {other:?}, wanted :logout, :user_switch, :suspend or :idle"
        ))),
    }
}

/// Ask the host not to do something while this application is busy: suspend,
/// idle, log out, switch user.
///
/// The inhibition lasts until the portal request is closed, which happens when
/// this process exits. There is deliberately no way to lift it earlier here;
/// that wants the request object, which the next version can carry.
fn inhibit(ruby: &Ruby, what: RArray, reason: String) -> Result<(), Error> {
    let mut flags = BitFlags::<InhibitFlags>::empty();

    for flag in what.into_iter() {
        flags |= inhibit_flag_from(&flag.to_r_string()?.to_string()?)?;
    }

    let _ = ruby;

    block(async {
        InhibitProxy::new()
            .await?
            .inhibit(
                None,
                flags,
                InhibitOptions::default().set_reason(reason.as_str()),
            )
            .await?
            .response()
    })?
    .map_err(rt_err)?;

    Ok(())
}

// --- org.freedesktop.portal.Secret -----------------------------------------

/// The master secret for this application, as a String of bytes.
///
/// This is not a keyring: it is one secret the host keeps for this application
/// alone, meant to be used as the key with which it encrypts its own storage.
/// A sandboxed application gets one it cannot read for any other, which is the
/// point.
fn secret_retrieve(ruby: &Ruby) -> Result<magnus::RString, Error> {
    let bytes = block(ashpd_core::desktop::secret::retrieve())?.map_err(rt_err)?;

    Ok(ruby.str_from_slice(&bytes))
}

pub fn init(namespace: RModule) -> Result<(), Error> {
    let email = namespace.define_module("Email")?;
    email.define_singleton_method("compose", function!(email_compose, 1))?;

    let background = namespace.define_module("Background")?;
    background.define_singleton_method("request", function!(background_request, 1))?;
    background.define_singleton_method("set_status", function!(background_set_status, 1))?;

    let notification = namespace.define_module("Notification")?;
    notification.define_singleton_method("add", function!(notification_add, 3))?;
    notification.define_singleton_method("remove", function!(notification_remove, 1))?;

    let wallpaper = namespace.define_module("Wallpaper")?;
    wallpaper.define_singleton_method("set", function!(wallpaper_set, 2))?;

    let inhibit_module = namespace.define_module("Inhibit")?;
    inhibit_module.define_singleton_method("inhibit", function!(inhibit, 2))?;

    let secret = namespace.define_module("Secret")?;
    secret.define_singleton_method("retrieve", function!(secret_retrieve, 0))?;

    let _ = RArray::try_convert;

    Ok(())
}
