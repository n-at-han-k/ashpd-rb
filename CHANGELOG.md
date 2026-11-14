# Changelog

## [Unreleased]

## [0.2.0] - 2026-11-14

### Added

- `Ashpd::ScreenCast::Session`, the first stateful portal: `new`,
  `select_sources`, `start`, `restore_token`, `open_pipe_wire_remote` and
  `close`. The session must be kept alive for as long as the stream is wanted,
  and `open_pipe_wire_remote` hands back the descriptor a PipeWire consumer
  needs to reach the node id that `start` returns. Alongside it, the plain
  queries `Ashpd::ScreenCast.available_source_types`,
  `Ashpd::ScreenCast.available_cursor_modes` and `Ashpd::ScreenCast.version`.
- `Ashpd::GlobalShortcuts::Session` for shortcuts that fire whether or not the
  application has focus: `new`, `bind`, `list`, `next_activated` and `close`.
  `bind` returns what the user agreed to, which is not necessarily what was
  asked for.
- `Ashpd::RemoteDesktop::Session` for driving someone's pointer and keyboard:
  `new`, `select_devices`, `start`, `restore_token`, `move_pointer`,
  `pointer_button`, `key`, `connect_to_eis` and `close`. Also the capability
  query `Ashpd::RemoteDesktop.available_device_types`.
- `Ashpd::Location::Session` for the user's position as a stream of updates:
  `new`, `start`, `next_location` and `close`. `next_location` takes a timeout
  in seconds because the first fix can be slow.
- `Ashpd::NetworkMonitor.available?`, `.metered?`, `.connectivity`, `.status`
  and `.can_reach?`. `connectivity` and the `"connectivity"` key in `status`
  come back as one of the symbols `:local`, `:limited`, `:captive_portal` or
  `:full_network`.
- `Ashpd::PowerProfileMonitor.power_saver_enabled?`, so a UI can do less when
  the host asks it to.
- `Ashpd::ProxyResolver.lookup`, which returns the proxies to try for a URI in
  order. `"direct://"` is the no-proxy answer.
- `Ashpd::Realtime.limits`, returning the host's priority, nice level and
  rt-time caps.
- `Ashpd::GameMode.active?`, `.status`, `.register` and `.unregister`.
  `.status` returns `:inactive`, `:active`, `:registered` or `:rejected`.
- `Ashpd::Email.compose` for opening the user's mail program on a draft.
- `Ashpd::Background.request` and `Ashpd::Background.set_status`. `request`
  returns `{"run_in_background" =>, "auto_start" =>}` — what the user granted.
- `Ashpd::Notification.add` and `Ashpd::Notification.remove`. Re-adding the
  same id replaces what is on screen rather than stacking another one.
- `Ashpd::Wallpaper.set` from a URI, with `:set_on` of `:lockscreen`,
  `:background` or `:both`.
- `Ashpd::Inhibit.inhibit`, taking any of `:logout`, `:user_switch`,
  `:suspend`, `:idle`. The inhibition lasts until the process exits; there is
  deliberately no release yet.
- `Ashpd::Secret.retrieve`, the single master secret this host keeps for the
  application, as a `String` of bytes.
- `Ashpd::FileChooser.open_file` and `Ashpd::FileChooser.save_file`, both
  returning a list of URIs (empty when the user cancelled).
- `Ashpd::DynamicLauncher.supported_launcher_types`,
  `Ashpd::DynamicLauncher.desktop_entry`, `Ashpd::DynamicLauncher.launch` and
  `Ashpd::DynamicLauncher.uninstall`.
- `Ashpd::OpenUri.scheme_supported?`, `Ashpd::OpenUri.open_path` (by path,
  through a descriptor) and `Ashpd::OpenUri.open_directory`.
- `Ashpd::Camera.request_access` and `Ashpd::Camera.open_pipe_wire_remote`.
  The descriptor only works while the grant is live, and Ruby owns it once
  returned.
- `Ashpd::Usb.devices`, the enumeration only — acquiring a device is not bound
  yet.
- `Ashpd::InputCapture.supported_capabilities`.
- `Ashpd::Settings.contrast` and `Ashpd::Settings.reduced_motion`.
- `Ashpd::Screenshot.available_targets`, as a list of `:screen`, `:window`,
  `:area` or `:active_window` symbols.
