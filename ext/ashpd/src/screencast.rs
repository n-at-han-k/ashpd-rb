//! org.freedesktop.portal.ScreenCast -- casting a screen to a PipeWire stream.
//!
//! This portal is unlike the others bound here, in two ways that shape the API.
//!
//! It is stateful. A cast is a *session*: created, then told what to record,
//! then started, and it lives until it is closed or the D-Bus connection that
//! made it goes away. So this is a class with methods rather than a module
//! function, and the object has to be kept for as long as the stream is wanted.
//! Let it go and the compositor stops casting.
//!
//! It hands back a file descriptor. The streams the portal publishes are not on
//! the PipeWire the rest of the session can see; they are on a remote of the
//! portal's own, and the only way in is a descriptor passed over D-Bus. A node
//! id alone is useless - a consumer that connects to the ordinary PipeWire and
//! asks for that node gets "target not found" - so {Session#open_pipe_wire_remote}
//! is what makes the stream readable, and the pair of numbers it leaves is what
//! a consumer needs:
//!
//! ```ruby
//! session = Ashpd::ScreenCast::Session.new
//! session.select_sources(sources: [:monitor], cursor_mode: :embedded)
//! stream = session.start.first
//! fd = session.open_pipe_wire_remote
//!
//! system("gst-launch-1.0", "pipewiresrc", "fd=#{fd}", "path=#{stream['node_id']}",
//!        "!", "videoconvert", "!", "autovideosink", fd => fd)
//! ```
//!
//! # Provenance
//!
//!   - <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html>
//!   - ashpd's own `screen_cast_gstreamer.rs` example, for the fd and node id pair.

use std::os::fd::IntoRawFd;
use std::sync::Mutex;

use magnus::{function, method, prelude::*, Error, RArray, RHash, RModule, Ruby, TryConvert};

use ashpd_core::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    StartCastOptions, Stream, Streams,
};
use ashpd_core::desktop::{PersistMode, Session};
use ashpd_core::enumflags2::BitFlags;

use crate::{block, option_bool, option_name, rt_err, runtime};

// --- names -----------------------------------------------------------------

/// The interfaces number their enumerations, and a number tells a reader
/// nothing. Each is taken and given as a symbol instead, with the name the
/// specification uses.
fn source_type_from(name: &str) -> Result<SourceType, Error> {
    match name {
        "monitor" => Ok(SourceType::Monitor),
        "window" => Ok(SourceType::Window),
        "virtual" => Ok(SourceType::Virtual),
        other => Err(rt_err(format!(
            "unknown source type {other:?}, wanted :monitor, :window or :virtual"
        ))),
    }
}

fn source_type_name(source_type: SourceType) -> &'static str {
    match source_type {
        SourceType::Monitor => "monitor",
        SourceType::Window => "window",
        SourceType::Virtual => "virtual",
    }
}

fn cursor_mode_from(name: &str) -> Result<CursorMode, Error> {
    match name {
        "hidden" => Ok(CursorMode::Hidden),
        "embedded" => Ok(CursorMode::Embedded),
        "metadata" => Ok(CursorMode::Metadata),
        other => Err(rt_err(format!(
            "unknown cursor mode {other:?}, wanted :hidden, :embedded or :metadata"
        ))),
    }
}

fn cursor_mode_name(cursor_mode: CursorMode) -> &'static str {
    match cursor_mode {
        CursorMode::Hidden => "hidden",
        CursorMode::Embedded => "embedded",
        CursorMode::Metadata => "metadata",
    }
}

/// How long the permission outlives the session: not at all, until the
/// application exits, or until the user revokes it. Only the last is any use to
/// a program that wants to cast again tomorrow without asking.
fn persist_mode_from(name: &str) -> Result<PersistMode, Error> {
    match name {
        "do_not" => Ok(PersistMode::DoNot),
        "application" => Ok(PersistMode::Application),
        "explicitly_revoked" => Ok(PersistMode::ExplicitlyRevoked),
        other => Err(rt_err(format!(
            "unknown persist mode {other:?}, wanted :do_not, :application or :explicitly_revoked"
        ))),
    }
}

// --- the session -----------------------------------------------------------

/// The proxy and the session together, because neither is any use alone and
/// both have to be let go at the same moment.
///
/// They are kept behind an Option so that closing can take them: a closed
/// session is a real state, and a method called on one should say so rather
/// than talk to a portal that has already finished with us.
struct Parts {
    proxy: Screencast,
    session: Session<Screencast>,
}

/// A screen cast session: what is being recorded, and the way in to it.
#[magnus::wrap(class = "Ashpd::ScreenCast::Session", free_immediately, size)]
struct ScreenCastSession {
    parts: Mutex<Option<Parts>>,
    /// What Start answered, kept so the streams and the restore token can be
    /// asked for separately afterwards.
    started: Mutex<Option<Streams>>,
}

impl ScreenCastSession {
    fn new() -> Result<Self, Error> {
        // One block_on for both, so that every temporary the proxy and the
        // session create along the way is dropped inside the runtime.
        let parts = block(async {
            let proxy = Screencast::new().await?;
            let session = proxy.create_session(Default::default()).await?;

            Ok::<_, ashpd_core::Error>(Parts { proxy, session })
        })?
        .map_err(rt_err)?;

        Ok(Self {
            parts: Mutex::new(Some(parts)),
            started: Mutex::new(None),
        })
    }

    /// Say what to record. This must come before {start}, and the portal allows
    /// it only once per session.
    ///
    /// `restore_token` is a token a previous session handed back. Given one the
    /// portal may restore that permission and start without asking the user
    /// again; a token it will not accept is ignored rather than fatal, and the
    /// user is asked as if none had been given.
    fn select_sources(ruby: &Ruby, rb_self: &Self, options: RHash) -> Result<(), Error> {
        let mut selection = SelectSourcesOptions::default();

        if let Some(sources) = options.get(ruby.to_symbol("sources")) {
            let sources = RArray::try_convert(sources)?;
            let mut types = BitFlags::<SourceType>::empty();

            for source in sources.into_iter() {
                types |= source_type_from(&source.to_r_string()?.to_string()?)?;
            }

            selection = selection.set_sources(types);
        }

        if let Some(name) = option_name(ruby, options, "cursor_mode")? {
            selection = selection.set_cursor_mode(cursor_mode_from(&name)?);
        }

        if let Some(multiple) = option_bool(ruby, options, "multiple") {
            selection = selection.set_multiple(multiple);
        }

        if let Some(name) = option_name(ruby, options, "persist_mode")? {
            selection = selection.set_persist_mode(persist_mode_from(&name)?);
        }

        let token = option_name(ruby, options, "restore_token")?;
        let selection = selection.set_restore_token(token.as_deref());

        let guard = rb_self.locked()?;
        let parts = Self::open(&guard)?;

        // The request is awaited and read inside the block, so that it is
        // dropped in there too. A zbus request holds tokio resources, and
        // dropping one with no runtime entered panics with "there is no
        // reactor running" -- which across the FFI boundary takes the VM with
        // it rather than raising.
        block(async {
            parts
                .proxy
                .select_sources(&parts.session, selection)
                .await?
                .response()
        })?
        .map_err(rt_err)?;

        Ok(())
    }

    /// Start casting, and return the streams the portal published.
    ///
    /// This is where the user is asked, unless a restore token spared them, so
    /// it does not return until they have answered.
    fn start(ruby: &Ruby, rb_self: &Self) -> Result<RArray, Error> {
        let streams = {
            let guard = rb_self.locked()?;
            let parts = Self::open(&guard)?;

            block(async {
                parts
                    .proxy
                    .start(&parts.session, None, StartCastOptions::default())
                    .await?
                    .response()
            })?
            .map_err(rt_err)?
        };

        let array = ruby.ary_new_capa(streams.streams().len());
        for stream in streams.streams() {
            let _ = array.push(stream_to(ruby, stream)?);
        }

        *rb_self.started.lock().map_err(rt_err)? = Some(streams);

        Ok(array)
    }

    /// The token to keep for next time, if the portal issued one.
    ///
    /// A fresh token comes back from every start, and using one does not keep
    /// it alive, so the one to save is always the newest.
    fn restore_token(&self) -> Result<Option<String>, Error> {
        Ok(self
            .started
            .lock()
            .map_err(rt_err)?
            .as_ref()
            .and_then(|streams| streams.restore_token().map(ToOwned::to_owned)))
    }

    /// A descriptor for the PipeWire remote the streams are on.
    ///
    /// The descriptor is handed to Ruby and Ruby owns it: nothing here closes
    /// it, and whoever asked should, once the stream is finished with. It only
    /// works while this session is alive.
    fn open_pipe_wire_remote(&self) -> Result<i32, Error> {
        let guard = self.locked()?;
        let parts = Self::open(&guard)?;

        let fd = block(async {
            parts
                .proxy
                .open_pipe_wire_remote(&parts.session, OpenPipeWireRemoteOptions::default())
                .await
        })?
        .map_err(rt_err)?;

        Ok(fd.into_raw_fd())
    }

    /// Stop casting, and let the portal go.
    ///
    /// Closing twice is not an error: the second time there is nothing left to
    /// close, which is the state the caller asked for.
    fn close(&self) -> Result<(), Error> {
        let Some(parts) = self.locked()?.take() else {
            return Ok(());
        };

        block(async {
            let result = parts.session.close().await;
            drop(parts);
            result
        })?
        .map_err(rt_err)
    }

    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Option<Parts>>, Error> {
        self.parts.lock().map_err(rt_err)
    }

    fn open<'a>(guard: &'a std::sync::MutexGuard<'_, Option<Parts>>) -> Result<&'a Parts, Error> {
        guard
            .as_ref()
            .ok_or_else(|| rt_err("the screen cast session is closed"))
    }
}

impl Drop for ScreenCastSession {
    /// Ruby frees this whenever the garbage collector gets to it, on whatever
    /// it was doing at the time. The parts have to be dropped inside the
    /// runtime for the same reason the requests above do, so the drop is done
    /// in a block rather than left to the fields.
    fn drop(&mut self) {
        let Ok(mut guard) = self.parts.lock() else {
            return;
        };
        let Some(parts) = guard.take() else {
            return;
        };
        let Ok(runtime) = runtime() else {
            return;
        };

        runtime.block_on(async move {
            drop(parts);
        });
    }
}

/// One stream, as `{"node_id" =>, "position" =>, "size" =>, "source_type" =>, "id" =>}`.
///
/// node_id is the only part a consumer must have; the rest the portal may omit,
/// and then it is nil.
fn stream_to(ruby: &Ruby, stream: &Stream) -> Result<RHash, Error> {
    let hash = ruby.hash_new();

    hash.aset("node_id", stream.pipe_wire_node_id())?;
    hash.aset("position", stream.position().map(|(x, y)| vec![x, y]))?;
    hash.aset("size", stream.size().map(|(width, height)| vec![width, height]))?;
    hash.aset(
        "source_type",
        stream.source_type().map(|kind| ruby.to_symbol(source_type_name(kind))),
    )?;
    hash.aset("id", stream.id())?;
    hash.aset("mapping_id", stream.mapping_id())?;

    Ok(hash)
}

// --- the portal ------------------------------------------------------------

/// Which kinds of source this compositor will cast, as symbols.
fn available_source_types(ruby: &Ruby) -> Result<RArray, Error> {
    let proxy = block(Screencast::new())?.map_err(rt_err)?;
    let types = block(proxy.available_source_types())?.map_err(rt_err)?;

    let array = ruby.ary_new();
    for source_type in types.iter() {
        let _ = array.push(ruby.to_symbol(source_type_name(source_type)));
    }

    Ok(array)
}

/// Which cursor modes this compositor offers, as symbols.
fn available_cursor_modes(ruby: &Ruby) -> Result<RArray, Error> {
    let proxy = block(Screencast::new())?.map_err(rt_err)?;
    let modes = block(proxy.available_cursor_modes())?.map_err(rt_err)?;

    let array = ruby.ary_new();
    for mode in modes.iter() {
        let _ = array.push(ruby.to_symbol(cursor_mode_name(mode)));
    }

    Ok(array)
}

/// The version of the interface the portal implements.
fn version() -> Result<u32, Error> {
    let proxy = block(Screencast::new())?.map_err(rt_err)?;

    Ok(proxy.version())
}

pub fn init(ruby: &Ruby, namespace: RModule) -> Result<(), Error> {
    let module = namespace.define_module("ScreenCast")?;

    module.define_singleton_method("available_source_types", function!(available_source_types, 0))?;
    module.define_singleton_method("available_cursor_modes", function!(available_cursor_modes, 0))?;
    module.define_singleton_method("version", function!(version, 0))?;

    let session = module.define_class("Session", ruby.class_object())?;
    session.define_singleton_method("new", function!(ScreenCastSession::new, 0))?;
    session.define_method("select_sources", method!(ScreenCastSession::select_sources, 1))?;
    session.define_method("start", method!(ScreenCastSession::start, 0))?;
    session.define_method("restore_token", method!(ScreenCastSession::restore_token, 0))?;
    session.define_method(
        "open_pipe_wire_remote",
        method!(ScreenCastSession::open_pipe_wire_remote, 0),
    )?;
    session.define_method("close", method!(ScreenCastSession::close, 0))?;

    Ok(())
}
