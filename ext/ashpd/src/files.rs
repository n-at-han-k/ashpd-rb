//! The portals that deal in files: choosing them, opening them, launching
//! things that handle them.
//!
//! The file chooser is the reason the portals exist at all. A sandboxed
//! application cannot see the user's files, so it asks the host to run the
//! dialog; what comes back is a URI the application has been granted, rather
//! than a path it guessed. Unsandboxed, these behave the same way, which is
//! what makes code written against them portable into a sandbox later.
//!
//!   - org.freedesktop.portal.FileChooser
//!   - org.freedesktop.portal.DynamicLauncher
//!   - org.freedesktop.portal.OpenURI (the parts beyond a plain URI)

use magnus::{function, prelude::*, Error, RArray, RHash, RModule, Ruby};

use ashpd_core::desktop::dynamic_launcher::{DynamicLauncherProxy, LaunchOptions, LauncherType, UninstallOptions};
use ashpd_core::desktop::file_chooser::{OpenFileRequest, SaveFileRequest, SelectedFiles};
use ashpd_core::desktop::open_uri::{OpenDirectoryRequest, OpenFileRequest as OpenUriFileRequest, SchemeSupportedOptions};
use ashpd_core::desktop::open_uri::OpenURIProxy;

use crate::{block, option_bool, option_name, rt_err};

/// The URIs a chooser came back with. Always a list, because a chooser that
/// allows several returns several and one that does not returns one - and a
/// caller that handles the list handles both.
fn uris_of(ruby: &Ruby, files: &SelectedFiles) -> RArray {
    let array = ruby.ary_new_capa(files.uris().len());
    for uri in files.uris() {
        let _ = array.push(uri.as_str());
    }
    array
}

// --- org.freedesktop.portal.FileChooser ------------------------------------

/// Ask the user to choose a file to open, returning the URIs chosen.
///
/// An empty list is not an error: it is the user saying no, which an
/// application has to handle anyway.
fn open_file(ruby: &Ruby, title: String, options: RHash) -> Result<RArray, Error> {
    let multiple = option_bool(ruby, options, "multiple");
    let directory = option_bool(ruby, options, "directory");
    let modal = option_bool(ruby, options, "modal");
    let accept_label = option_name(ruby, options, "accept_label")?;

    let files = block(async {
        OpenFileRequest::default()
            .title(title.as_str())
            .accept_label(accept_label.as_deref())
            .modal(modal)
            .multiple(multiple)
            .directory(directory)
            .send()
            .await?
            .response()
    })?
    .map_err(rt_err)?;

    Ok(uris_of(ruby, &files))
}

/// Ask the user where to save something, returning the URI chosen.
fn save_file(ruby: &Ruby, title: String, options: RHash) -> Result<RArray, Error> {
    let modal = option_bool(ruby, options, "modal");
    let accept_label = option_name(ruby, options, "accept_label")?;
    let current_name = option_name(ruby, options, "current_name")?;

    let files = block(async {
        SaveFileRequest::default()
            .title(title.as_str())
            .accept_label(accept_label.as_deref())
            .modal(modal)
            .current_name(current_name.as_deref())
            .send()
            .await?
            .response()
    })?
    .map_err(rt_err)?;

    Ok(uris_of(ruby, &files))
}

// --- org.freedesktop.portal.OpenURI ----------------------------------------

/// Whether the host has anything that opens this kind of URI, which is worth
/// knowing before offering the user a button that would do nothing.
fn scheme_supported(scheme: String) -> Result<bool, Error> {
    block(async {
        OpenURIProxy::new()
            .await?
            .scheme_supported(&scheme, SchemeSupportedOptions::default())
            .await
    })?
    .map_err(rt_err)
}

/// Open a local file with whatever handles it.
///
/// The interface takes a descriptor rather than a path, so the file is opened
/// here - read-only, because opening is all the handler needs and asking for
/// write would fail on a file the user can read but not change.
fn open_path(path: String, ask: bool) -> Result<(), Error> {
    let file = std::fs::File::open(&path).map_err(rt_err)?;

    block(async {
        OpenUriFileRequest::default()
            .ask(ask)
            .send_file(&file)
            .await?
            .response()
    })?
    .map_err(rt_err)?;

    Ok(())
}

/// Open a directory in the user's file manager, with the entry selected.
fn open_directory(path: String) -> Result<(), Error> {
    let directory = std::fs::File::open(&path).map_err(rt_err)?;

    block(async {
        OpenDirectoryRequest::default()
            .send(&directory)
            .await?
            .response()
    })?
    .map_err(rt_err)?;

    Ok(())
}

// --- org.freedesktop.portal.DynamicLauncher --------------------------------

fn launcher_type_name(launcher_type: LauncherType) -> &'static str {
    match launcher_type {
        LauncherType::Application => "application",
        LauncherType::WebApplication => "web_application",
    }
}

/// Which kinds of launcher this host will install, as symbols.
fn supported_launcher_types(ruby: &Ruby) -> Result<RArray, Error> {
    let types = block(async {
        DynamicLauncherProxy::new()
            .await?
            .supported_launcher_types()
            .await
    })?
    .map_err(rt_err)?;

    let array = ruby.ary_new();
    for launcher_type in types.iter() {
        let _ = array.push(ruby.to_symbol(launcher_type_name(launcher_type)));
    }

    Ok(array)
}

/// The desktop entry of a launcher this application installed, as the text of
/// the file.
fn desktop_entry(desktop_file_id: String) -> Result<String, Error> {
    block(async {
        DynamicLauncherProxy::new()
            .await?
            .desktop_entry(&desktop_file_id)
            .await
    })?
    .map_err(rt_err)
}

/// Launch something this application installed earlier.
fn launch(desktop_file_id: String) -> Result<(), Error> {
    block(async {
        DynamicLauncherProxy::new()
            .await?
            .launch(&desktop_file_id, LaunchOptions::default())
            .await
    })?
    .map_err(rt_err)
}

/// Remove a launcher this application installed. The portal only allows an
/// application to remove its own.
fn uninstall(desktop_file_id: String) -> Result<(), Error> {
    block(async {
        DynamicLauncherProxy::new()
            .await?
            .uninstall(&desktop_file_id, UninstallOptions::default())
            .await
    })?
    .map_err(rt_err)
}

pub fn init(namespace: RModule) -> Result<(), Error> {
    let chooser = namespace.define_module("FileChooser")?;
    chooser.define_singleton_method("open_file", function!(open_file, 2))?;
    chooser.define_singleton_method("save_file", function!(save_file, 2))?;

    let open_uri = namespace.define_module("OpenUri")?;
    open_uri.define_singleton_method("scheme_supported?", function!(scheme_supported, 1))?;
    open_uri.define_singleton_method("open_path", function!(open_path, 2))?;
    open_uri.define_singleton_method("open_directory", function!(open_directory, 1))?;

    let launcher = namespace.define_module("DynamicLauncher")?;
    launcher.define_singleton_method("supported_launcher_types", function!(supported_launcher_types, 0))?;
    launcher.define_singleton_method("desktop_entry", function!(desktop_entry, 1))?;
    launcher.define_singleton_method("launch", function!(launch, 1))?;
    launcher.define_singleton_method("uninstall", function!(uninstall, 1))?;

    Ok(())
}
