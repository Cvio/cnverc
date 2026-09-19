//! Paired mode (SPEC §9): two cnverc instances on a local network as the two
//! ends of one conversation.
//!
//! Each machine runs its own complete pipeline and sends the other only the
//! translated **text**. The receiver shows it and speaks it with its own
//! voice. No audio, no models and no inference cross the wire.
//!
//! Everything here runs on its own threads (SPEC §11), so a stalled or dead
//! peer can never block local capture, recognition or the window:
//!
//! * the **peer thread** owns the connection, the handshake and the floor
//!   token, and is the only thing that writes to the socket;
//! * an **acceptor** waits for the other side to dial in;
//! * a **dialler** makes one outgoing attempt when the user presses Connect;
//! * a **reader** per connection turns lines into messages.
//!
//! Only IP addresses are accepted. A name would be resolved by the system's
//! resolver, which can mean a public DNS server, and §2 forbids that.
//! Everything works on a cable or a switch with no router, no DHCP and no DNS.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use tracing::{debug, info, warn};

use crate::config::{Config, ModeKind};
use crate::discovery::{self, Found};
use crate::floor::{self, Action, Floor, Holder};
use crate::pipeline::{PipelineCmd, PipelineMsg, SpeakJob};
use crate::wire::{self, Wire, WireError, MAX_LINE, MAX_NAME};

/// The port §7 listens on by default, used when an address is typed without
/// one.
pub const DEFAULT_PORT: u16 = 47800;

/// How long a dial may take before it counts as unanswered.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);

/// How often a live connection is pinged.
const PING_EVERY: Duration = Duration::from_secs(2);

/// Silence after which the other side is taken to be gone. A pulled cable
/// sends nothing at all, not even a reset; this is how it is noticed.
const DEAD_AFTER: Duration = Duration::from_secs(6);

/// How long a connection may go without saying `Hello`.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// A write that takes longer than this means the link is gone.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// How often the peer thread wakes with nothing to do, for the timers.
const TICK: Duration = Duration::from_millis(100);

/// What the rest of cnverc can ask of the peer thread.
#[derive(Debug, Clone, PartialEq)]
pub enum PeerCmd {
    Connect(SocketAddr),
    Disconnect,
    /// The turn key asked for a turn. In turn mode, while connected, this
    /// asks for the floor; otherwise the turn starts at once.
    WantTurn,
    /// The turn key ended the turn, or cancelled a request still in flight.
    EndTurn,
    SetMode(ModeKind),
    /// A translated utterance to send.
    Deliver(Outgoing),
    /// The turn's utterance has been sent, or there was none. Hand back the
    /// floor.
    ReleaseFloor,
}

/// One translated utterance on its way to the other side.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    /// The local caption it belongs to.
    pub index: usize,
    pub lang: String,
    pub text: String,
    pub source_lang: String,
    pub source_text: String,
}

/// The connection, as the window shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum PeerState {
    /// Listening, not connected.
    Waiting {
        port: u16,
    },
    Connecting(SocketAddr),
    Connected {
        name: String,
        addr: SocketAddr,
        speaks: String,
        sends: String,
    },
    /// Was connected, or tried to be, and is not. Still listening.
    Disconnected {
        port: u16,
        reason: String,
    },
    /// Paired mode cannot work at all: the port could not be opened.
    Unavailable(String),
}

/// A way to reach the peer thread. Cheap to clone.
#[derive(Clone)]
pub struct PeerHandle {
    tx: Sender<Input>,
}

impl PeerHandle {
    pub fn send(&self, command: PeerCmd) {
        let _ = self.tx.send(Input::Cmd(command));
    }
}

/// Who this PC is, as `Hello` says it.
#[derive(Debug, Clone, PartialEq)]
pub struct Me {
    pub name: String,
    pub speaks: String,
    pub sends: String,
}

impl Me {
    pub fn from_config(config: &Config) -> Self {
        Self {
            name: display_name(&config.peer.display_name),
            speaks: config.languages.source.clone(),
            sends: config.languages.target.clone(),
        }
    }
}

/// `[peer].display_name`, or the computer's name when it is empty.
pub fn display_name(configured: &str) -> String {
    let name = if configured.trim().is_empty() {
        gethostname::gethostname().to_string_lossy().into_owned()
    } else {
        configured.trim().to_string()
    };
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    if name.is_empty() {
        "cnverc".to_string()
    } else {
        name
    }
}

/// Everything the peer thread hears, from every other thread.
enum Input {
    Cmd(PeerCmd),
    Accepted(TcpStream, SocketAddr),
    Dialled(SocketAddr, std::io::Result<TcpStream>),
    Line(u64, Result<Wire, WireError>),
    Closed(u64, String),
    Found(Vec<Found>),
}

/// Where the peer thread sends things.
pub struct Wiring {
    /// Straight to the pipeline: a granted floor opens the microphone without
    /// a round trip through the window.
    pub pipeline: Sender<PipelineCmd>,
    /// Received utterances to speak.
    pub speak: SyncSender<SpeakJob>,
    pub events: Sender<PipelineMsg>,
}

/// Start listening, and the thread that runs paired mode. Returns at once; a
/// port that cannot be opened is reported as [`PeerState::Unavailable`], not
/// as an error, so the rest of cnverc still runs.
pub fn start(
    config: &Config,
    stop: Arc<AtomicBool>,
    wiring: Wiring,
) -> Result<(PeerHandle, JoinHandle<()>)> {
    let (tx, rx) = channel();
    let me = Me::from_config(config);
    let listen_addr = config.peer.listen_addr.trim().to_string();
    let discovery_on = config.peer.discovery;
    let mode = config.mode.kind;

    let handle = PeerHandle { tx: tx.clone() };
    let thread = std::thread::Builder::new()
        .name("cnverc-peer".to_string())
        .spawn(move || {
            let mut peer = Peer::new(me, mode, wiring, tx.clone());
            let helpers = peer.listen(&listen_addr, discovery_on, &stop, &tx);
            peer.run(&rx, &stop);
            peer.shut_down();
            for helper in helpers {
                let _ = helper.join();
            }
            info!("paired mode stopped");
        })
        .context("cannot spawn the peer thread")?;
    Ok((handle, thread))
}

/// Parse what the user typed as the other PC's address. An IP address, with
/// the port optional. Names are refused rather than looked up (SPEC §2).
pub fn parse_address(text: &str) -> Result<SocketAddr> {
    let text = text.trim();
    if text.is_empty() {
        return Err(anyhow!("type the other PC's address, e.g. 192.168.50.2"));
    }
    if let Ok(addr) = text.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Ok(ip) = text.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    Err(anyhow!(
        "\"{text}\" is not an IP address. Type the numbers shown on the other PC, e.g. \
         192.168.50.2 (names are never looked up, because that could need the internet)"
    ))
}

/// One TCP connection, before or after `Hello`.
struct Link {
    id: u64,
    stream: TcpStream,
    local: SocketAddr,
    remote: SocketAddr,
    /// Whether this PC dialled it.
    outbound: bool,
    hello: Option<Me>,
    opened: Instant,
    last_ping: Instant,
}

impl Link {
    /// The end that dialled. Two connections between the same pair are told
    /// apart by it, and both ends see it the same way round.
    fn dialler(&self) -> SocketAddr {
        if self.outbound {
            self.local
        } else {
            self.remote
        }
    }

    fn name(&self) -> String {
        self.hello
            .as_ref()
            .map(|h| h.name.clone())
            .unwrap_or_else(|| self.remote.ip().to_string())
    }

    fn write(&mut self, message: &Wire) -> std::io::Result<()> {
        self.stream.write_all(wire::encode(message).as_bytes())
    }
}

struct Peer {
    me: Me,
    mode: ModeKind,
    wiring: Wiring,
    tx: Sender<Input>,
    port: u16,
    links: Vec<Link>,
    active: Option<u64>,
    floor: Option<Floor>,
    next_id: u64,
    next_seq: u64,
    /// Whether anyone has ever connected in. Part of the firewall diagnostic
    /// (SPEC §10).
    ever_accepted: bool,
    dialling: Option<SocketAddr>,
    listening: bool,
}

impl Peer {
    fn new(me: Me, mode: ModeKind, wiring: Wiring, tx: Sender<Input>) -> Self {
        Self {
            me,
            mode,
            wiring,
            tx,
            port: 0,
            links: Vec::new(),
            active: None,
            floor: None,
            next_id: 1,
            next_seq: 1,
            ever_accepted: false,
            dialling: None,
            listening: false,
        }
    }

    fn emit(&self, msg: PipelineMsg) {
        let _ = self.wiring.events.send(msg);
    }

    fn state(&self, state: PeerState) {
        self.emit(PipelineMsg::Peer(state));
    }

    /// Bind the listener and start the helper threads.
    fn listen(
        &mut self,
        listen_addr: &str,
        discovery_on: bool,
        stop: &Arc<AtomicBool>,
        tx: &Sender<Input>,
    ) -> Vec<JoinHandle<()>> {
        let mut helpers = Vec::new();
        let listener = listen_addr
            .parse::<SocketAddr>()
            .map_err(|_| {
                anyhow!("[peer].listen_addr = \"{listen_addr}\" is not an address and port")
            })
            .and_then(|addr| {
                TcpListener::bind(addr).with_context(|| {
                    format!(
                        "cannot listen on {addr}. Is another cnverc already running on this PC? \
                         Two on one PC need different [peer].listen_addr ports"
                    )
                })
            });
        let listener = match listener {
            Ok(listener) => listener,
            Err(e) => {
                warn!("{e:#}");
                self.state(PeerState::Unavailable(format!("{e:#}")));
                return helpers;
            }
        };
        self.port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
        self.listening = true;
        info!(
            "paired mode: listening on port {} as \"{}\"",
            self.port, self.me.name
        );
        self.state(PeerState::Waiting { port: self.port });

        match spawn_acceptor(listener, stop.clone(), tx.clone()) {
            Ok(thread) => helpers.push(thread),
            Err(e) => warn!("{e:#}"),
        }

        // Discovery is convenience only (SPEC §9): if it cannot start, typing
        // the address still works, so a failure is logged and nothing more.
        if discovery_on {
            let found_tx = tx.clone();
            match discovery::start(
                &self.me.name,
                self.port,
                stop.clone(),
                Box::new(move |found| {
                    let _ = found_tx.send(Input::Found(found));
                }),
            ) {
                Ok(threads) => helpers.extend(threads),
                Err(e) => warn!("discovery is off: {e:#}"),
            }
        }
        helpers
    }

    fn run(&mut self, rx: &Receiver<Input>, stop: &AtomicBool) {
        while !stop.load(Ordering::Relaxed) {
            match rx.recv_timeout(TICK) {
                Ok(input) => self.handle(input),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.timers();
        }
    }

    fn handle(&mut self, input: Input) {
        match input {
            Input::Cmd(command) => self.command(command),
            Input::Accepted(stream, remote) => {
                self.ever_accepted = true;
                info!("paired mode: {remote} connected in");
                self.add_link(stream, false);
            }
            Input::Dialled(addr, result) => {
                if self.dialling != Some(addr) {
                    return;
                }
                self.dialling = None;
                match result {
                    Ok(stream) => {
                        info!("paired mode: connected out to {addr}");
                        self.add_link(stream, true);
                    }
                    Err(e) => {
                        let reason = diagnose(&e, addr, self.ever_accepted, self.port);
                        warn!("paired mode: cannot connect to {addr}: {e}");
                        self.state(PeerState::Disconnected {
                            port: self.port,
                            reason,
                        });
                    }
                }
            }
            Input::Line(id, Ok(message)) => self.receive(id, message),
            Input::Line(id, Err(error)) => {
                let from = self.link_name(id);
                warn!("paired mode: from {from}: {error}");
                self.emit(PipelineMsg::Error(format!("From {from}: {error}")));
                // A version mismatch or a runaway line cannot be recovered
                // from on this connection. Anything else is skipped, which is
                // what makes a hand-typed netcat session usable.
                if matches!(error, WireError::UnknownProto(_) | WireError::TooLong) {
                    self.close(
                        id,
                        Some(error.to_string()),
                        format!("{from} could not be understood: {error}"),
                    );
                }
            }
            Input::Closed(id, reason) => {
                let name = self.link_name(id);
                self.drop_link(id, format!("{name} {reason}"));
            }
            Input::Found(found) => self.emit(PipelineMsg::Discovered(found)),
        }
    }

    fn command(&mut self, command: PeerCmd) {
        match command {
            PeerCmd::Connect(addr) => {
                if let Some(link) = self.active_link() {
                    self.emit(PipelineMsg::Error(format!(
                        "Already paired with {}. Disconnect first.",
                        link.name()
                    )));
                    return;
                }
                if !self.listening {
                    self.emit(PipelineMsg::Error(
                        "Paired mode is unavailable on this PC; see the peer panel".to_string(),
                    ));
                    return;
                }
                self.dialling = Some(addr);
                self.state(PeerState::Connecting(addr));
                let tx = self.tx.clone();
                let spawned = std::thread::Builder::new()
                    .name("cnverc-dial".to_string())
                    .spawn(move || {
                        let result = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT);
                        let _ = tx.send(Input::Dialled(addr, result));
                    });
                if let Err(e) = spawned {
                    self.dialling = None;
                    self.emit(PipelineMsg::Error(format!("cannot dial {addr}: {e}")));
                }
            }
            PeerCmd::Disconnect => {
                self.dialling = None;
                if let Some(id) = self.active {
                    self.close(id, None, "you disconnected".to_string());
                } else {
                    self.state(PeerState::Waiting { port: self.port });
                }
            }
            PeerCmd::WantTurn => {
                let asked = match (self.floor.as_mut(), self.mode) {
                    (Some(floor), ModeKind::Turn) => Some(floor.request(Instant::now())),
                    _ => None,
                };
                match asked {
                    Some(actions) => self.act(actions),
                    // Not connected, or continuous: nobody to ask.
                    None => {
                        let _ = self.wiring.pipeline.send(PipelineCmd::BeginTurn);
                    }
                }
            }
            PeerCmd::EndTurn => {
                if let Some(floor) = &mut self.floor {
                    let actions = floor.cancel();
                    self.act(actions);
                }
                let _ = self.wiring.pipeline.send(PipelineCmd::EndTurn);
            }
            PeerCmd::SetMode(mode) => self.mode = mode,
            PeerCmd::Deliver(out) => self.deliver(out),
            PeerCmd::ReleaseFloor => {
                if let Some(floor) = &mut self.floor {
                    let actions = floor.release();
                    self.act(actions);
                }
            }
        }
    }

    fn deliver(&mut self, out: Outgoing) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let message = Wire::Utterance {
            seq,
            lang: out.lang,
            text: out.text,
            source_lang: out.source_lang,
            source_text: out.source_text,
        };
        let Some(id) = self.active else {
            self.emit(PipelineMsg::NotSent {
                index: out.index,
                reason: "not connected to another PC".to_string(),
            });
            return;
        };
        match self.write_to(id, &message) {
            Some(to) => {
                debug!("paired mode: utterance {} sent to {to}", out.index);
                self.emit(PipelineMsg::Sent {
                    index: out.index,
                    to,
                });
            }
            None => self.emit(PipelineMsg::NotSent {
                index: out.index,
                reason: "the connection dropped".to_string(),
            }),
        }
    }

    fn receive(&mut self, id: u64, message: Wire) {
        if let Wire::Hello {
            name,
            speaks,
            sends,
            ..
        } = &message
        {
            let hello = Me {
                name: name.clone(),
                speaks: speaks.clone(),
                sends: sends.clone(),
            };
            self.hello(id, hello);
            return;
        }
        if let Wire::Bye { reason } = &message {
            let name = self.link_name(id);
            let why = match reason {
                Some(r) => format!("{name} ended the connection: {r}"),
                None => format!("{name} ended the connection"),
            };
            self.drop_link(id, why);
            return;
        }
        if self.active != Some(id) {
            // Nothing but Hello and Bye counts before the handshake.
            return;
        }
        match message {
            Wire::Utterance {
                lang,
                text,
                source_lang,
                source_text,
                ..
            } => {
                let from = self.link_name(id);
                info!(
                    "paired mode: from {from}\n  [{source_lang}] {source_text}\n  [{lang}] {text}"
                );
                self.emit(PipelineMsg::Remote {
                    from: from.clone(),
                    lang: lang.clone(),
                    text: text.clone(),
                    source_lang,
                    source_text,
                });
                match self.wiring.speak.try_send(SpeakJob {
                    index: None,
                    lang,
                    text,
                    since: Instant::now(),
                }) {
                    Ok(()) => {}
                    // No speaker: speech is off. Captions are enough.
                    Err(TrySendError::Disconnected(_)) => {}
                    Err(TrySendError::Full(_)) => self.emit(PipelineMsg::Error(format!(
                        "too much arrived from {from} at once; an utterance was shown but not \
                         spoken"
                    ))),
                }
            }
            Wire::FloorRequest { .. } | Wire::FloorGrant { .. } | Wire::FloorRelease { .. } => {
                if let Some(floor) = &mut self.floor {
                    let actions = floor.receive(&message);
                    self.act(actions);
                }
            }
            Wire::Ping => {
                self.write_to(id, &Wire::Pong);
            }
            Wire::Pong | Wire::Hello { .. } | Wire::Bye { .. } => {}
        }
    }

    /// A connection has said who it is: make it the peer, or refuse it.
    fn hello(&mut self, id: u64, hello: Me) {
        let Some(index) = self.links.iter().position(|l| l.id == id) else {
            return;
        };
        if self.links[index].hello.is_some() {
            return;
        }
        self.links[index].hello = Some(hello.clone());

        if let Some(active) = self.active {
            let (keep_new, same_peer) = {
                let current = self.links.iter().find(|l| l.id == active);
                let new = &self.links[index];
                match current {
                    // Both ends dialled at once: two connections to the same
                    // PC. Both ends keep the one with the lower dialling
                    // address, so they settle on the same one.
                    Some(current)
                        if current.remote.ip() == new.remote.ip()
                            && current.hello.as_ref().map(|h| &h.name) == Some(&hello.name) =>
                    {
                        (new.dialler() < current.dialler(), true)
                    }
                    _ => (false, false),
                }
            };
            if same_peer {
                debug!(
                    "paired mode: two connections to {}; keeping one",
                    hello.name
                );
                if keep_new {
                    self.active = None;
                    self.floor = None;
                    self.discard(active, "duplicate connection");
                    self.activate(id);
                } else {
                    self.discard(id, "duplicate connection");
                }
                return;
            }
            // Exactly one peer at a time (SPEC §9). The newcomer is told why.
            let current = self.active_link().map(Link::name).unwrap_or_default();
            let reason = format!("{} is already paired with {current}", self.me.name);
            warn!("paired mode: refused {}: {reason}", hello.name);
            self.emit(PipelineMsg::Error(format!(
                "Refused a connection from {}: already paired with {current}",
                hello.name
            )));
            self.discard(id, &reason);
            return;
        }

        self.activate(id);
    }

    /// Make a connection that has said `Hello` the peer.
    fn activate(&mut self, id: u64) {
        let Some(link) = self.links.iter().find(|l| l.id == id) else {
            return;
        };
        let Some(hello) = link.hello.clone() else {
            return;
        };
        let wins = floor::wins_ties(
            &self.me.name,
            &hello.name,
            &link.local.to_string(),
            &link.remote.to_string(),
        );
        let addr = link.remote;
        self.active = Some(id);
        self.dialling = None;
        self.floor = Some(Floor::new(&hello.name, wins));
        info!(
            "paired mode: paired with \"{}\" at {addr}, who speaks {} and sends {}",
            hello.name, hello.speaks, hello.sends
        );
        self.state(PeerState::Connected {
            name: hello.name.clone(),
            addr,
            speaks: hello.speaks.clone(),
            sends: hello.sends.clone(),
        });
        self.emit(PipelineMsg::FloorChanged(Holder::Free));

        // What they send is what this PC will speak. Say so if it is not the
        // language this PC's person speaks; it is not an error, and the
        // receiving voice is chosen by each utterance's own language anyway.
        if hello.sends != self.me.speaks {
            self.emit(PipelineMsg::Error(format!(
                "{} translates into {}, but this PC is set up for someone who speaks {}. \
                 Check the languages on both PCs.",
                hello.name, hello.sends, self.me.speaks
            )));
        }
    }

    /// Carry out what the floor decided.
    fn act(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Send(message) => {
                    if let Some(id) = self.active {
                        self.write_to(id, &message);
                    }
                }
                Action::BeginTurn => {
                    let _ = self.wiring.pipeline.send(PipelineCmd::BeginTurn);
                }
                Action::EndTurn => {
                    let _ = self.wiring.pipeline.send(PipelineCmd::EndTurn);
                }
                Action::Refused(why) => {
                    info!("paired mode: turn refused: {why}");
                    self.emit(PipelineMsg::FloorRefused(why));
                }
                Action::Changed(holder) => self.emit(PipelineMsg::FloorChanged(holder)),
            }
        }
    }

    fn timers(&mut self) {
        let now = Instant::now();
        if let Some(floor) = &mut self.floor {
            let actions = floor.tick(now);
            self.act(actions);
        }

        let mut stale = Vec::new();
        let mut ping = Vec::new();
        for link in &self.links {
            if link.hello.is_none() && now.duration_since(link.opened) > HELLO_TIMEOUT {
                stale.push(link.id);
            } else if Some(link.id) == self.active
                && now.duration_since(link.last_ping) >= PING_EVERY
            {
                ping.push(link.id);
            }
        }
        for id in stale {
            let addr = self.link_name(id);
            self.close(
                id,
                Some("no Hello".to_string()),
                format!("{addr} connected but never said who it was; is it cnverc?"),
            );
        }
        for id in ping {
            if let Some(link) = self.links.iter_mut().find(|l| l.id == id) {
                link.last_ping = now;
            }
            self.write_to(id, &Wire::Ping);
        }
    }

    fn add_link(&mut self, stream: TcpStream, outbound: bool) {
        let (Ok(local), Ok(remote)) = (stream.local_addr(), stream.peer_addr()) else {
            return;
        };
        let _ = stream.set_nodelay(true);
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let reader = match stream.try_clone() {
            Ok(reader) => reader,
            Err(e) => {
                warn!("paired mode: cannot read from {remote}: {e}");
                return;
            }
        };
        let id = self.next_id;
        self.next_id += 1;
        let now = Instant::now();
        let mut link = Link {
            id,
            stream,
            local,
            remote,
            outbound,
            hello: None,
            opened: now,
            last_ping: now,
        };
        let hello = Wire::Hello {
            name: self.me.name.clone(),
            speaks: self.me.speaks.clone(),
            sends: self.me.sends.clone(),
            proto: wire::PROTO,
        };
        if let Err(e) = link.write(&hello) {
            warn!("paired mode: cannot greet {remote}: {e}");
            return;
        }
        if let Err(e) = spawn_reader(id, reader, self.tx.clone()) {
            warn!("{e:#}");
            return;
        }
        self.links.push(link);
    }

    /// Write to one connection. A failed write closes it. Returns the other
    /// side's name when the write went out.
    fn write_to(&mut self, id: u64, message: &Wire) -> Option<String> {
        let link = self.links.iter_mut().find(|l| l.id == id)?;
        match link.write(message) {
            Ok(()) => Some(link.name()),
            Err(e) => {
                let name = link.name();
                self.drop_link(id, format!("{name} stopped answering ({e})"));
                None
            }
        }
    }

    /// Say goodbye on a connection and close it.
    fn close(&mut self, id: u64, reason: Option<String>, why: String) {
        if let Some(link) = self.links.iter_mut().find(|l| l.id == id) {
            let _ = link.write(&Wire::Bye { reason });
        }
        self.drop_link(id, why);
    }

    /// Close a connection that is not the peer, quietly.
    fn discard(&mut self, id: u64, reason: &str) {
        if let Some(index) = self.links.iter().position(|l| l.id == id) {
            let mut link = self.links.remove(index);
            let _ = link.write(&Wire::Bye {
                reason: Some(reason.to_string()),
            });
            let _ = link.stream.shutdown(Shutdown::Both);
        }
    }

    /// A connection is gone. If it was the peer, the floor is force-released
    /// and the window is told (SPEC §9).
    fn drop_link(&mut self, id: u64, why: String) {
        let Some(index) = self.links.iter().position(|l| l.id == id) else {
            return;
        };
        let link = self.links.remove(index);
        let _ = link.stream.shutdown(Shutdown::Both);

        if self.active == Some(id) {
            self.active = None;
            let mut floor = self.floor.take();

            // Both ends dialled at once, and the other end closed the spare
            // connection first: carry on over the one that is left.
            let spare = self
                .links
                .iter()
                .find(|l| {
                    l.remote.ip() == link.remote.ip()
                        && l.hello.is_some()
                        && l.hello.as_ref().map(|h| &h.name) == link.hello.as_ref().map(|h| &h.name)
                })
                .map(|l| l.id);
            if let Some(spare) = spare {
                debug!("paired mode: carrying on over the other connection");
                self.activate(spare);
                return;
            }

            warn!("paired mode: disconnected: {why}");
            // The disconnected state first, so the window never shows a free
            // floor on a link that is already gone.
            self.state(PeerState::Disconnected {
                port: self.port,
                reason: why,
            });
            if let Some(floor) = floor.as_mut() {
                let actions = floor.disconnected();
                self.act(actions);
            }
        } else if link.outbound && link.hello.is_none() {
            // Our own dial, refused before it was ever a pairing.
            self.state(PeerState::Disconnected {
                port: self.port,
                reason: why,
            });
        }
    }

    fn active_link(&self) -> Option<&Link> {
        self.links.iter().find(|l| Some(l.id) == self.active)
    }

    fn link_name(&self, id: u64) -> String {
        self.links
            .iter()
            .find(|l| l.id == id)
            .map(Link::name)
            .unwrap_or_else(|| "the other PC".to_string())
    }

    fn shut_down(&mut self) {
        let ids: Vec<u64> = self.links.iter().map(|l| l.id).collect();
        for id in ids {
            if let Some(link) = self.links.iter_mut().find(|l| l.id == id) {
                let _ = link.write(&Wire::Bye {
                    reason: Some("cnverc was stopped".to_string()),
                });
                let _ = link.stream.shutdown(Shutdown::Both);
            }
        }
        self.links.clear();
    }
}

/// Wait for the other side to dial in. Non-blocking, so it notices the stop.
fn spawn_acceptor(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    tx: Sender<Input>,
) -> Result<JoinHandle<()>> {
    listener
        .set_nonblocking(true)
        .context("cannot make the listener non-blocking")?;
    std::thread::Builder::new()
        .name("cnverc-accept".to_string())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, remote)) => {
                        let _ = stream.set_nonblocking(false);
                        if tx.send(Input::Accepted(stream, remote)).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        warn!("paired mode: accept failed: {e}");
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        })
        .context("cannot spawn the acceptor thread")
}

/// Turn one connection's bytes into messages. Ends when the connection does.
fn spawn_reader(id: u64, stream: TcpStream, tx: Sender<Input>) -> Result<JoinHandle<()>> {
    // Anything at all, a Pong included, arrives at least every PING_EVERY on
    // a live link. A read that waits longer than DEAD_AFTER is a dead link.
    stream
        .set_read_timeout(Some(DEAD_AFTER))
        .context("cannot set a read timeout")?;
    std::thread::Builder::new()
        .name("cnverc-peer-read".to_string())
        .spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = Vec::with_capacity(512);
            loop {
                line.clear();
                // One byte past the limit, so an over-long line is seen as
                // such rather than silently split in two.
                let read = reader
                    .by_ref()
                    .take(MAX_LINE as u64 + 1)
                    .read_until(b'\n', &mut line);
                let reason = match read {
                    Ok(0) => "closed the connection".to_string(),
                    Ok(_) if !line.ends_with(b"\n") && line.len() > MAX_LINE => {
                        let _ = tx.send(Input::Line(id, Err(WireError::TooLong)));
                        "sent a message that was too long".to_string()
                    }
                    Ok(_) => {
                        if line.iter().all(u8::is_ascii_whitespace) {
                            continue;
                        }
                        if tx.send(Input::Line(id, wire::decode(&line))).is_err() {
                            return;
                        }
                        continue;
                    }
                    Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                        format!(
                            "went silent: nothing heard for {} s. Was a cable pulled?",
                            DEAD_AFTER.as_secs()
                        )
                    }
                    Err(e) => format!("dropped the connection ({e})"),
                };
                let _ = tx.send(Input::Closed(id, reason));
                return;
            }
        })
        .context("cannot spawn a reader thread")
}

/// Explain a failed dial in terms of what to do about it (SPEC §10).
///
/// A refusal means the packets arrived and nothing was listening. A timeout
/// means they were silently dropped, which on a cable or a router-less switch
/// is nearly always the Windows firewall treating an unidentified network as
/// Public.
fn diagnose(error: &std::io::Error, addr: SocketAddr, ever_accepted: bool, port: u16) -> String {
    match error.kind() {
        ErrorKind::ConnectionRefused => format!(
            "Nothing is listening at {addr}. On that PC, is cnverc running with \"Pair with \
             another PC\" ticked and Start pressed? Is {} the port it shows?",
            addr.port()
        ),
        ErrorKind::TimedOut | ErrorKind::WouldBlock => {
            let mut text = format!(
                "No answer from {addr} within {} s. The connection is being silently dropped, \
                 which is almost always a firewall. On Windows, a cable or a switch with no \
                 router makes a network Windows cannot identify, and it usually calls it \
                 Public, which blocks incoming connections. On the other PC, open Settings › \
                 Network & internet, choose this network's adapter, and set its network \
                 profile to Private, or allow cnverc through Windows Defender Firewall. On \
                 Linux, allow TCP port {} (ufw allow {}/tcp).",
                CONNECT_TIMEOUT.as_secs(),
                addr.port(),
                addr.port()
            );
            if !ever_accepted {
                text.push_str(&format!(
                    " This PC has not received a connection either since it started listening \
                     on port {port}, so check this PC's firewall and network profile too."
                ));
            }
            text
        }
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => format!(
            "There is no route to {addr}. Are both PCs on the same cable, switch or Wi-Fi \
             network, and is the address typed exactly as the other PC shows it?"
        ),
        ErrorKind::AddrNotAvailable => format!(
            "{addr} cannot be dialled from this PC. Type the address the other PC shows under \
             \"This PC\"."
        ),
        _ => format!("Cannot connect to {addr}: {error}"),
    }
}

/// This PC's network addresses, for the other person to type in. Loopback is
/// left out; nobody else can reach it.
pub fn local_addresses() -> Vec<(String, IpAddr)> {
    let mut found: Vec<(String, IpAddr)> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter(|i| match i.ip() {
            IpAddr::V4(_) => true,
            // IPv6 link-local addresses need a zone to be dialled, and a
            // person cannot be expected to type one.
            IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
        })
        .map(|i| (i.name.clone(), i.ip()))
        .collect();
    // IPv4 first: it is what people type.
    found.sort_by_key(|(name, ip)| (ip.is_ipv6(), name.clone()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;

    #[test]
    fn an_address_is_an_ip_with_an_optional_port_and_never_a_name() {
        assert_eq!(
            parse_address("192.168.50.2").expect("ip"),
            "192.168.50.2:47800".parse().expect("addr")
        );
        assert_eq!(
            parse_address(" 10.0.0.7:47802 ").expect("ip:port"),
            "10.0.0.7:47802".parse().expect("addr")
        );
        assert!(
            parse_address("laptop-b.local").is_err(),
            "names are not resolved"
        );
        assert!(parse_address("").is_err());
    }

    #[test]
    fn a_timeout_names_the_firewall_and_the_network_profile() {
        let addr: SocketAddr = "192.168.50.2:47800".parse().expect("addr");
        let timed_out = std::io::Error::from(ErrorKind::TimedOut);
        let text = diagnose(&timed_out, addr, false, 47800);
        assert!(
            text.contains("firewall") && text.contains("Private"),
            "{text}"
        );
        assert!(
            text.contains("This PC has not received a connection"),
            "{text}"
        );

        let refused = std::io::Error::from(ErrorKind::ConnectionRefused);
        assert!(diagnose(&refused, addr, true, 47800).contains("Nothing is listening"));
    }

    /// Two ends on this PC, over loopback, one in each direction. What SPEC §9
    /// asks for, without a microphone or a model: the handshake, an
    /// utterance each way, the floor, and a dropped connection force-releasing
    /// the floor on both ends.
    struct End {
        handle: PeerHandle,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
        events: Receiver<PipelineMsg>,
        pipeline: Receiver<PipelineCmd>,
        speak: Receiver<SpeakJob>,
    }

    impl End {
        fn start(name: &str, speaks: &str, sends: &str, port: u16) -> Self {
            let mut config = Config::default();
            config.peer.enabled = true;
            config.peer.display_name = name.to_string();
            config.peer.listen_addr = format!("127.0.0.1:{port}");
            config.peer.discovery = false;
            config.languages.source = speaks.to_string();
            config.languages.target = sends.to_string();
            config.mode.kind = ModeKind::Turn;
            let (events_tx, events) = channel();
            let (pipeline_tx, pipeline) = channel();
            let (speak_tx, speak) = sync_channel(8);
            let stop = Arc::new(AtomicBool::new(false));
            let (handle, thread) = start(
                &config,
                stop.clone(),
                Wiring {
                    pipeline: pipeline_tx,
                    speak: speak_tx,
                    events: events_tx,
                },
            )
            .expect("start");
            Self {
                handle,
                stop,
                thread: Some(thread),
                events,
                pipeline,
                speak,
            }
        }

        fn wait_for(&self, what: &str, want: impl Fn(&PipelineMsg) -> bool) -> PipelineMsg {
            let deadline = Instant::now() + Duration::from_secs(10);
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                match self.events.recv_timeout(left) {
                    Ok(msg) if want(&msg) => return msg,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            panic!("never saw {what}");
        }

        fn stop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    impl Drop for End {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn connected(msg: &PipelineMsg) -> bool {
        matches!(msg, PipelineMsg::Peer(PeerState::Connected { .. }))
    }

    #[test]
    fn two_ends_pair_talk_take_turns_and_notice_a_drop() {
        let a = End::start("laptop-a", "es", "en", 47_911);
        let mut b = End::start("laptop-b", "en", "es", 47_912);
        a.wait_for("a listening", |m| {
            matches!(m, PipelineMsg::Peer(PeerState::Waiting { .. }))
        });
        b.wait_for("b listening", |m| {
            matches!(m, PipelineMsg::Peer(PeerState::Waiting { .. }))
        });

        a.handle
            .send(PeerCmd::Connect("127.0.0.1:47912".parse().expect("addr")));
        let PipelineMsg::Peer(PeerState::Connected { name, sends, .. }) =
            a.wait_for("a connected", connected)
        else {
            unreachable!()
        };
        assert_eq!((name.as_str(), sends.as_str()), ("laptop-b", "es"));
        b.wait_for("b connected", connected);

        // A takes a turn: the microphone opens only on the grant.
        a.handle.send(PeerCmd::WantTurn);
        a.wait_for("a holds the floor", |m| {
            matches!(m, PipelineMsg::FloorChanged(Holder::Me))
        });
        assert_eq!(
            a.pipeline.recv_timeout(Duration::from_secs(5)),
            Ok(PipelineCmd::BeginTurn)
        );
        b.wait_for(
            "b sees a has the floor",
            |m| matches!(m, PipelineMsg::FloorChanged(Holder::Them(n)) if n == "laptop-a"),
        );

        // B presses its key while A talks: refused, microphone stays shut.
        b.handle.send(PeerCmd::WantTurn);
        b.wait_for("b refused", |m| matches!(m, PipelineMsg::FloorRefused(_)));
        assert!(
            b.pipeline.recv_timeout(Duration::from_millis(300)).is_err(),
            "b opened its microphone while a held the floor"
        );

        // A's utterance arrives at B as text in B's language, to be spoken.
        a.handle.send(PeerCmd::Deliver(Outgoing {
            index: 1,
            lang: "en".into(),
            text: "Where is the station?".into(),
            source_lang: "es".into(),
            source_text: "¿Dónde está la estación?".into(),
        }));
        a.wait_for("a sent", |m| {
            matches!(m, PipelineMsg::Sent { index: 1, .. })
        });
        let PipelineMsg::Remote { from, text, .. } =
            b.wait_for("b received", |m| matches!(m, PipelineMsg::Remote { .. }))
        else {
            unreachable!()
        };
        assert_eq!(
            (from.as_str(), text.as_str()),
            ("laptop-a", "Where is the station?")
        );
        let job = b
            .speak
            .recv_timeout(Duration::from_secs(5))
            .expect("spoken");
        assert_eq!(
            job.lang, "en",
            "the voice follows the utterance's own language"
        );

        // A hands the floor back; B may now talk.
        a.handle.send(PeerCmd::ReleaseFloor);
        b.wait_for("b sees the floor free", |m| {
            matches!(m, PipelineMsg::FloorChanged(Holder::Free))
        });
        b.handle.send(PeerCmd::WantTurn);
        b.wait_for("b holds the floor", |m| {
            matches!(m, PipelineMsg::FloorChanged(Holder::Me))
        });
        a.wait_for("a sees b has the floor", |m| {
            matches!(m, PipelineMsg::FloorChanged(Holder::Them(_)))
        });

        // B goes away mid-turn. A is told, and the floor is free again.
        b.stop();
        a.wait_for("a disconnected", |m| {
            matches!(m, PipelineMsg::Peer(PeerState::Disconnected { .. }))
        });
        a.wait_for("a's floor force-released", |m| {
            matches!(m, PipelineMsg::FloorChanged(Holder::Free))
        });
    }

    #[test]
    fn a_second_connection_is_refused_with_a_reason() {
        let a = End::start("laptop-a", "es", "en", 47_921);
        let b = End::start("laptop-b", "en", "es", 47_922);
        let c = End::start("laptop-c", "en", "es", 47_923);
        a.handle
            .send(PeerCmd::Connect("127.0.0.1:47922".parse().expect("addr")));
        a.wait_for("a connected", connected);
        b.wait_for("b connected", connected);

        c.handle
            .send(PeerCmd::Connect("127.0.0.1:47922".parse().expect("addr")));
        let PipelineMsg::Peer(PeerState::Disconnected { reason, .. }) = c
            .wait_for("c refused", |m| {
                matches!(m, PipelineMsg::Peer(PeerState::Disconnected { .. }))
            })
        else {
            unreachable!()
        };
        assert!(reason.contains("already paired with laptop-a"), "{reason}");
        b.wait_for(
            "b says it refused",
            |m| matches!(m, PipelineMsg::Error(e) if e.contains("laptop-c")),
        );
    }

    #[test]
    fn dialling_each_other_at_once_ends_with_one_pairing() {
        let a = End::start("laptop-a", "es", "en", 47_931);
        let b = End::start("laptop-b", "en", "es", 47_932);
        a.handle
            .send(PeerCmd::Connect("127.0.0.1:47932".parse().expect("addr")));
        b.handle
            .send(PeerCmd::Connect("127.0.0.1:47931".parse().expect("addr")));
        a.wait_for("a connected", connected);
        b.wait_for("b connected", connected);

        // Whichever connection survived, it carries utterances both ways.
        std::thread::sleep(Duration::from_millis(500));
        for (from, to) in [(&a, &b), (&b, &a)] {
            from.handle.send(PeerCmd::Deliver(Outgoing {
                index: 1,
                lang: "en".into(),
                text: "hello".into(),
                source_lang: "es".into(),
                source_text: "hola".into(),
            }));
            to.wait_for("the utterance", |m| matches!(m, PipelineMsg::Remote { .. }));
        }
    }

    #[test]
    fn a_hand_typed_line_is_understood_and_nonsense_is_reported() {
        // What netcat would do (SPEC §9): connect, say hello, send a line.
        let a = End::start("laptop-a", "es", "en", 47_941);
        a.wait_for("listening", |m| {
            matches!(m, PipelineMsg::Peer(PeerState::Waiting { .. }))
        });
        let mut nc = TcpStream::connect("127.0.0.1:47941").expect("connect");
        nc.write_all(
            b"{\"t\":\"Hello\",\"name\":\"netcat\",\"speaks\":\"en\",\"sends\":\"es\",\"proto\":1}\n\
              this is not json\n\
              {\"t\":\"Utterance\",\"seq\":1,\"lang\":\"es\",\"text\":\"Hola\",\"source_lang\":\"en\",\"source_text\":\"Hi\"}\n",
        )
        .expect("write");
        a.wait_for("nonsense reported", |m| matches!(m, PipelineMsg::Error(_)));
        a.wait_for(
            "the utterance",
            |m| matches!(m, PipelineMsg::Remote { text, .. } if text == "Hola"),
        );

        nc.write_all(
            b"{\"t\":\"Hello\",\"name\":\"x\",\"speaks\":\"en\",\"sends\":\"es\",\"proto\":9}\n",
        )
        .expect("write");
        a.wait_for(
            "the version named",
            |m| matches!(m, PipelineMsg::Error(e) if e.contains("version 9")),
        );
    }
}
