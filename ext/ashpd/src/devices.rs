//! The portals that hand over a device, or say what devices there are.
//!
//! Two of these answer with a file descriptor rather than data - the camera's
//! PipeWire remote, and remote desktop's EIS socket - and the rule for both is
//! the one {crate::screencast} documents: the descriptor only works while the
//! session that produced it is alive, and Ruby owns it once it is handed over.
//!
//!   - org.freedesktop.portal.Camera
//!   - org.freedesktop.portal.Usb
//!   - org.freedesktop.portal.RemoteDesktop (what it can do)
//!   - org.freedesktop.portal.InputCapture (what it can do)

use std::os::fd::IntoRawFd;

use magnus::{function, prelude::*, Error, RArray, RHash, RModule, Ruby};

use ashpd_core::desktop::camera::{Camera, CameraAccessOptions, OpenPipeWireRemoteOptions};
use ashpd_core::desktop::input_capture::{Capabilities, InputCapture};
use ashpd_core::desktop::remote_desktop::{DeviceType, RemoteDesktop};
use ashpd_core::desktop::usb::{EnumerateDevicesOptions, UsbProxy};

use crate::{block, rt_err};

// --- org.freedesktop.portal.Camera -----------------------------------------

/// Ask the user for the camera.
///
/// Returns true if there is now access. The dialog is shown once and the
/// answer remembered, so a second call on a granted camera returns at once.
fn camera_request_access() -> Result<bool, Error> {
    block(async {
        Camera::new()
            .await?
            .request_access(CameraAccessOptions::default())
            .await?
            .response()
    })?
    .map_err(rt_err)?;

    Ok(true)
}

/// A descriptor for the PipeWire remote the cameras are on.
///
/// As with screen casting, the node to read is not addressable on the ordinary
/// PipeWire: this descriptor is the way in. Access must have been granted
/// first, so call {camera_request_access} before this.
fn camera_open_pipe_wire_remote() -> Result<i32, Error> {
    let fd = block(async {
        Camera::new()
            .await?
            .open_pipe_wire_remote(OpenPipeWireRemoteOptions::default())
            .await
    })?
    .map_err(rt_err)?;

    Ok(fd.into_raw_fd())
}

// --- org.freedesktop.portal.Usb --------------------------------------------

/// Every USB device the host will tell us about, as
/// `[{"id" =>, "vendor" =>, "model" =>, "device_file" =>, "readable" =>, "writable" =>}]`.
///
/// This is the enumeration only, which shows the user nothing. Acquiring a
/// device - which hands back a descriptor and does ask - needs a session, and
/// is not bound yet.
fn usb_devices(ruby: &Ruby) -> Result<RArray, Error> {
    let devices = block(async {
        UsbProxy::new()
            .await?
            .enumerate_devices(EnumerateDevicesOptions::default())
            .await
    })?
    .map_err(rt_err)?;

    let array = ruby.ary_new_capa(devices.len());
    for (id, device) in devices {
        let hash = ruby.hash_new();
        hash.aset("id", id.as_str())?;
        hash.aset("vendor", device.vendor())?;
        hash.aset("model", device.model())?;
        hash.aset("device_file", device.device_file())?;
        hash.aset("readable", device.is_readable())?;
        hash.aset("writable", device.is_writable())?;
        let _ = array.push(hash);
    }

    Ok(array)
}

// --- what the input portals can do -----------------------------------------

/// Which input devices this host will let an application drive remotely, as
/// `[:keyboard, :pointer, :touchscreen]`.
fn remote_desktop_device_types(ruby: &Ruby) -> Result<RArray, Error> {
    let types = block(async { RemoteDesktop::new().await?.available_device_types().await })?
        .map_err(rt_err)?;

    let array = ruby.ary_new();
    for device_type in types.iter() {
        let _ = array.push(ruby.to_symbol(match device_type {
            DeviceType::Keyboard => "keyboard",
            DeviceType::Pointer => "pointer",
            DeviceType::Touchscreen => "touchscreen",
        }));
    }

    Ok(array)
}

/// Which kinds of input this host will let an application capture at a screen
/// edge, as `[:keyboard, :pointer, :touchscreen]`.
fn input_capture_capabilities(ruby: &Ruby) -> Result<RArray, Error> {
    let capabilities = block(async { InputCapture::new().await?.supported_capabilities().await })?
        .map_err(rt_err)?;

    let array = ruby.ary_new();
    for capability in capabilities.iter() {
        let _ = array.push(ruby.to_symbol(match capability {
            Capabilities::Keyboard => "keyboard",
            Capabilities::Pointer => "pointer",
            Capabilities::Touchscreen => "touchscreen",
        }));
    }

    Ok(array)
}

pub fn init(namespace: RModule) -> Result<(), Error> {
    let camera = namespace.define_module("Camera")?;
    camera.define_singleton_method("request_access", function!(camera_request_access, 0))?;
    camera.define_singleton_method(
        "open_pipe_wire_remote",
        function!(camera_open_pipe_wire_remote, 0),
    )?;

    let usb = namespace.define_module("Usb")?;
    usb.define_singleton_method("devices", function!(usb_devices, 0))?;

    let remote_desktop = namespace.define_module("RemoteDesktop")?;
    remote_desktop
        .define_singleton_method("available_device_types", function!(remote_desktop_device_types, 0))?;

    let input_capture = namespace.define_module("InputCapture")?;
    input_capture
        .define_singleton_method("supported_capabilities", function!(input_capture_capabilities, 0))?;

    let _: Option<RHash> = None;

    Ok(())
}
