//! The portals that answer questions about the machine, rather than asking the
//! user for something.
//!
//! None of these shows a dialog, so unlike the rest of this binding they are
//! safe to call from a main loop: they are property reads and plain method
//! calls. That makes them the right things to branch a user interface on -
//! whether there is a network, whether the machine is on battery - before
//! asking for anything that interrupts.
//!
//!   - org.freedesktop.portal.NetworkMonitor
//!   - org.freedesktop.portal.PowerProfileMonitor
//!   - org.freedesktop.portal.ProxyResolver
//!   - org.freedesktop.portal.Realtime
//!   - org.freedesktop.portal.GameMode

use magnus::{function, prelude::*, Error, RArray, RHash, RModule, Ruby};

use ashpd_core::desktop::game_mode::{GameMode, Status};
use ashpd_core::desktop::network_monitor::{Connectivity, NetworkMonitor};
use ashpd_core::desktop::power_profile_monitor::PowerProfileMonitor;
use ashpd_core::desktop::proxy_resolver::ProxyResolver;
use ashpd_core::desktop::realtime::Realtime;
use ashpd_core::Uri;

use crate::{block, rt_err};

// --- org.freedesktop.portal.NetworkMonitor ---------------------------------

/// How much of a network there is, as the interface's own four states rather
/// than a boolean: a captive portal is connected and useless, which is a
/// distinction an application wants to draw.
fn connectivity_name(connectivity: Connectivity) -> &'static str {
    match connectivity {
        Connectivity::Local => "local",
        Connectivity::Limited => "limited",
        Connectivity::CaptivePortal => "captive_portal",
        Connectivity::FullNetwork => "full_network",
    }
}

/// Whether there is any network at all.
fn network_available() -> Result<bool, Error> {
    block(async { NetworkMonitor::new().await?.is_available().await })?.map_err(rt_err)
}

/// Whether the network bills by the byte, which is what to check before
/// downloading something large on someone else's behalf.
fn network_metered() -> Result<bool, Error> {
    block(async { NetworkMonitor::new().await?.is_metered().await })?.map_err(rt_err)
}

fn network_connectivity(ruby: &Ruby) -> Result<magnus::Symbol, Error> {
    let connectivity =
        block(async { NetworkMonitor::new().await?.connectivity().await })?.map_err(rt_err)?;

    Ok(ruby.to_symbol(connectivity_name(connectivity)))
}

/// The three answers at once, in one round trip rather than three.
fn network_status(ruby: &Ruby) -> Result<RHash, Error> {
    let status = block(async { NetworkMonitor::new().await?.status().await })?.map_err(rt_err)?;

    let hash = ruby.hash_new();
    hash.aset("available", status.is_available())?;
    hash.aset("metered", status.is_metered())?;
    hash.aset(
        "connectivity",
        ruby.to_symbol(connectivity_name(status.connectivity())),
    )?;

    Ok(hash)
}

/// Whether a host and port could be reached.
///
/// This asks the portal, not the network: it answers from what the host
/// already knows about routing, and opens no connection of its own.
fn network_can_reach(hostname: String, port: u32) -> Result<bool, Error> {
    block(async {
        NetworkMonitor::new()
            .await?
            .can_reach(&hostname, port)
            .await
    })?
    .map_err(rt_err)
}

// --- org.freedesktop.portal.PowerProfileMonitor ----------------------------

/// Whether power saving is on, which is the host asking applications to do
/// less: fewer animations, less background work.
fn power_saver_enabled() -> Result<bool, Error> {
    block(async { PowerProfileMonitor::new().await?.is_enabled().await })?.map_err(rt_err)
}

// --- org.freedesktop.portal.ProxyResolver ----------------------------------

/// The proxies to use for a URI, in the order to try them.
///
/// "direct://" means no proxy, and is what comes back when none is configured
/// - so the list is never empty and the first entry is the one to use.
fn proxy_lookup(ruby: &Ruby, uri: String) -> Result<RArray, Error> {
    let parsed = Uri::parse(&uri).map_err(rt_err)?;
    let proxies = block(async { ProxyResolver::new().await?.lookup(&parsed).await })?
        .map_err(rt_err)?;

    let array = ruby.ary_new_capa(proxies.len());
    for proxy in proxies {
        let _ = array.push(proxy.as_str());
    }

    Ok(array)
}

// --- org.freedesktop.portal.Realtime ---------------------------------------

/// What the host will allow a thread to ask for, as
/// `{"max_realtime_priority" =>, "min_nice_level" =>, "rt_time_usec_max" =>}`.
///
/// Only the limits are read here. Making a thread realtime is deliberately not
/// bound: it takes a thread id, and a Ruby thread is not the kernel thread it
/// runs on for long enough to make that meaningful.
fn realtime_limits(ruby: &Ruby) -> Result<RHash, Error> {
    let hash = ruby.hash_new();

    // Each property is read on its own and a failure leaves nil behind, rather
    // than one unimplemented property taking the other two with it. A portal
    // is free to carry some of an interface and not the rest.
    let max_priority = block(async { Realtime::new().await?.max_realtime_priority().await })?;
    let min_nice = block(async { Realtime::new().await?.min_nice_level().await })?;
    let rt_time = block(async { Realtime::new().await?.rt_time_usec_max().await })?;

    hash.aset("max_realtime_priority", max_priority.ok())?;
    hash.aset("min_nice_level", min_nice.ok())?;
    hash.aset("rt_time_usec_max", rt_time.ok())?;

    Ok(hash)
}

// --- org.freedesktop.portal.GameMode ---------------------------------------

/// Whether game mode is on for the host as a whole.
fn game_mode_active() -> Result<bool, Error> {
    block(async { GameMode::new().await?.is_active().await })?.map_err(rt_err)
}

/// Whether one process is registered with game mode, as `:inactive`,
/// `:active` or `:registered`.
fn game_mode_status(ruby: &Ruby, pid: u32) -> Result<magnus::Symbol, Error> {
    // The by-pid form, which names the asking process as well as the asked
    // about one. The single-argument QueryStatus wants to work out the caller's
    // pid namespace for itself and fails outside a sandbox with "Could not get
    // pidns".
    let status = block(async {
        GameMode::new()
            .await?
            .query_status_by_pid(pid, std::process::id())
            .await
    })?
    .map_err(rt_err)?;

    Ok(ruby.to_symbol(match status {
        Status::Inactive => "inactive",
        Status::Active => "active",
        Status::Registered => "registered",
        // The host refused: game mode exists but will not have this process.
        Status::Rejected => "rejected",
    }))
}

/// Ask the host to treat a process as a game: it may raise its priority and
/// stop power saving from throttling it.
///
/// The requester is this process, which is what the portal wants in order to
/// unregister on our behalf if we go away.
fn game_mode_register(pid: u32) -> Result<(), Error> {
    block(async {
        GameMode::new()
            .await?
            .register_by_pid(pid, std::process::id())
            .await
    })?
    .map_err(rt_err)
}

fn game_mode_unregister(pid: u32) -> Result<(), Error> {
    block(async {
        GameMode::new()
            .await?
            .unregister_by_pid(pid, std::process::id())
            .await
    })?
    .map_err(rt_err)
}

pub fn init(namespace: RModule) -> Result<(), Error> {
    let network = namespace.define_module("NetworkMonitor")?;
    network.define_singleton_method("available?", function!(network_available, 0))?;
    network.define_singleton_method("metered?", function!(network_metered, 0))?;
    network.define_singleton_method("connectivity", function!(network_connectivity, 0))?;
    network.define_singleton_method("status", function!(network_status, 0))?;
    network.define_singleton_method("can_reach?", function!(network_can_reach, 2))?;

    let power = namespace.define_module("PowerProfileMonitor")?;
    power.define_singleton_method("power_saver_enabled?", function!(power_saver_enabled, 0))?;

    let proxy = namespace.define_module("ProxyResolver")?;
    proxy.define_singleton_method("lookup", function!(proxy_lookup, 1))?;

    let realtime = namespace.define_module("Realtime")?;
    realtime.define_singleton_method("limits", function!(realtime_limits, 0))?;

    let game_mode = namespace.define_module("GameMode")?;
    game_mode.define_singleton_method("active?", function!(game_mode_active, 0))?;
    game_mode.define_singleton_method("status", function!(game_mode_status, 1))?;
    game_mode.define_singleton_method("register", function!(game_mode_register, 1))?;
    game_mode.define_singleton_method("unregister", function!(game_mode_unregister, 1))?;

    Ok(())
}
