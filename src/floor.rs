//! The floor token (SPEC §9): in turn-based paired mode exactly one peer holds
//! the floor, and the microphone is live only on that peer.
//!
//! This is the rule by itself, with no sockets and no clock of its own, so
//! every case is tested directly: what each event does to the floor, and what
//! must be sent or done as a result. The peer thread feeds it events and
//! carries out the [`Action`]s.
//!
//! The rules, all from §9:
//!
//! * Pressing the turn key sends `FloorRequest`. The microphone opens only
//!   when `FloorGrant` arrives.
//! * Two requests at once: the peer whose name sorts first wins, and the other
//!   is told the other party has the floor. No negotiation.
//! * No grant within two seconds fails the turn visibly. The microphone never
//!   opens on a timeout.
//! * The floor is released after the turn's utterance has been sent, and
//!   force-released if the connection drops.

use std::time::{Duration, Instant};

use crate::wire::Wire;

/// How long to wait for `FloorGrant` (SPEC §9).
pub const GRANT_TIMEOUT: Duration = Duration::from_secs(2);

/// Who holds the floor, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    /// Nobody: either side may take a turn.
    Free,
    /// This PC has asked and is waiting for the other side's answer.
    Asking,
    /// This PC: its microphone is live.
    Me,
    /// The other PC, by name: this PC's microphone stays closed.
    Them(String),
}

/// Something the peer thread must do because the floor changed.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Send(Wire),
    /// The floor is ours: open the microphone.
    BeginTurn,
    /// The floor has gone: close the microphone and finish the turn.
    EndTurn,
    /// A turn was asked for and will not happen. Shown on screen.
    Refused(String),
    /// Who holds the floor has changed.
    Changed(Holder),
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    Free,
    Asking {
        seq: u64,
        since: Instant,
        /// The key was released before the answer came. A grant that
        /// arrives now is handed straight back.
        cancelled: bool,
    },
    Mine {
        seq: u64,
    },
    Theirs {
        seq: u64,
    },
}

/// The floor, as this PC sees it.
#[derive(Debug)]
pub struct Floor {
    state: State,
    next_seq: u64,
    /// Whether this PC wins when both ask at once: its name sorts first.
    wins_ties: bool,
    them: String,
}

impl Floor {
    /// A free floor between this PC and `them`. `wins_ties` is whether this
    /// PC's name sorts before theirs; see [`wins_ties`].
    pub fn new(them: &str, wins_ties: bool) -> Self {
        Self {
            state: State::Free,
            next_seq: 1,
            wins_ties,
            them: them.to_string(),
        }
    }

    pub fn holder(&self) -> Holder {
        match self.state {
            State::Free => Holder::Free,
            State::Asking { .. } => Holder::Asking,
            State::Mine { .. } => Holder::Me,
            State::Theirs { .. } => Holder::Them(self.them.clone()),
        }
    }

    /// The turn key was pressed.
    pub fn request(&mut self, now: Instant) -> Vec<Action> {
        match self.state {
            State::Free => {
                let seq = self.take_seq();
                self.state = State::Asking {
                    seq,
                    since: now,
                    cancelled: false,
                };
                vec![
                    Action::Send(Wire::FloorRequest { seq }),
                    Action::Changed(Holder::Asking),
                ]
            }
            State::Theirs { .. } => vec![Action::Refused(format!(
                "{} has the floor. Wait for them to finish.",
                self.them
            ))],
            // Already asking, or already ours.
            State::Asking { .. } | State::Mine { .. } => Vec::new(),
        }
    }

    /// The turn key was released, or pressed again, before the answer came.
    pub fn cancel(&mut self) -> Vec<Action> {
        if let State::Asking { cancelled, .. } = &mut self.state {
            *cancelled = true;
        }
        Vec::new()
    }

    /// The turn's utterance has been sent, or there was none to send.
    pub fn release(&mut self) -> Vec<Action> {
        match self.state {
            State::Mine { seq } => {
                self.state = State::Free;
                vec![
                    Action::Send(Wire::FloorRelease { seq }),
                    Action::Changed(Holder::Free),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// Check the grant timeout.
    pub fn tick(&mut self, now: Instant) -> Vec<Action> {
        match self.state {
            State::Asking {
                since, cancelled, ..
            } if now.duration_since(since) >= GRANT_TIMEOUT => {
                self.state = State::Free;
                let mut actions = vec![Action::Changed(Holder::Free)];
                if !cancelled {
                    actions.push(Action::Refused(format!(
                        "{} did not answer within {} s, so the microphone stayed closed. \
                         Try again.",
                        self.them,
                        GRANT_TIMEOUT.as_secs()
                    )));
                }
                actions
            }
            _ => Vec::new(),
        }
    }

    /// A message about the floor arrived from the other side.
    pub fn receive(&mut self, message: &Wire) -> Vec<Action> {
        match *message {
            Wire::FloorRequest { seq } => self.their_request(seq),
            Wire::FloorGrant { seq } => self.their_grant(seq),
            Wire::FloorRelease { seq } => match self.state {
                State::Theirs { seq: held } if held == seq => {
                    self.state = State::Free;
                    vec![Action::Changed(Holder::Free)]
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// The connection has gone: the floor is force-released (SPEC §9).
    pub fn disconnected(&mut self) -> Vec<Action> {
        let before = std::mem::replace(&mut self.state, State::Free);
        match before {
            State::Free => Vec::new(),
            State::Mine { .. } => vec![Action::EndTurn, Action::Changed(Holder::Free)],
            State::Asking { cancelled, .. } => {
                let mut actions = vec![Action::Changed(Holder::Free)];
                if !cancelled {
                    actions.push(Action::Refused(
                        "the connection dropped before the floor was granted".to_string(),
                    ));
                }
                actions
            }
            State::Theirs { .. } => vec![Action::Changed(Holder::Free)],
        }
    }

    fn their_request(&mut self, seq: u64) -> Vec<Action> {
        let grant = |floor: &mut Self| {
            floor.state = State::Theirs { seq };
            Action::Send(Wire::FloorGrant { seq })
        };
        match self.state {
            State::Free | State::Theirs { .. } => {
                vec![grant(self), Action::Changed(self.holder())]
            }
            // Both asked at once. The winner ignores the other's request and
            // waits for its own grant, which the loser is about to send.
            State::Asking { cancelled, .. } => {
                if self.wins_ties && !cancelled {
                    Vec::new()
                } else {
                    let mut actions = vec![grant(self), Action::Changed(self.holder())];
                    if !cancelled {
                        actions.push(Action::Refused(format!(
                            "{} asked at the same moment and has the floor.",
                            self.them
                        )));
                    }
                    actions
                }
            }
            // We are talking. No answer: they time out and are told.
            State::Mine { .. } => Vec::new(),
        }
    }

    fn their_grant(&mut self, seq: u64) -> Vec<Action> {
        match self.state {
            State::Asking {
                seq: asked,
                cancelled: false,
                ..
            } if asked == seq => {
                self.state = State::Mine { seq };
                vec![Action::BeginTurn, Action::Changed(Holder::Me)]
            }
            // The same grant twice: the turn is already ours.
            State::Mine { seq: held } if held == seq => Vec::new(),
            // A grant nobody is waiting for: cancelled, timed out, or stale.
            // The other side now believes we hold the floor, so hand it back
            // at once rather than leave both ends disagreeing.
            _ => {
                let was_asking =
                    matches!(self.state, State::Asking { seq: asked, .. } if asked == seq);
                if was_asking {
                    self.state = State::Free;
                }
                let mut actions = vec![Action::Send(Wire::FloorRelease { seq })];
                if was_asking {
                    actions.push(Action::Changed(Holder::Free));
                }
                actions
            }
        }
    }

    fn take_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }
}

/// Whether this PC wins a simultaneous request: its name sorts first (SPEC
/// §9). Two PCs with the same name fall back to their addresses, which the two
/// ends see the same way round, so they still agree.
pub fn wins_ties(me: &str, them: &str, my_addr: &str, their_addr: &str) -> bool {
    (me, my_addr) < (them, their_addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Instant {
        // One fixed origin, so "later" is well defined within a test.
        static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *ORIGIN.get_or_init(Instant::now) + Duration::from_millis(ms)
    }

    fn sent(actions: &[Action]) -> Vec<Wire> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Send(w) => Some(w.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_microphone_opens_only_when_the_grant_arrives() {
        let mut floor = Floor::new("laptop-b", true);
        let asked = floor.request(at(0));
        assert_eq!(sent(&asked), vec![Wire::FloorRequest { seq: 1 }]);
        assert!(
            !asked.contains(&Action::BeginTurn),
            "opened before the grant"
        );
        assert_eq!(floor.holder(), Holder::Asking);

        let granted = floor.receive(&Wire::FloorGrant { seq: 1 });
        assert!(granted.contains(&Action::BeginTurn));
        assert_eq!(floor.holder(), Holder::Me);

        let released = floor.release();
        assert_eq!(sent(&released), vec![Wire::FloorRelease { seq: 1 }]);
        assert_eq!(floor.holder(), Holder::Free);
    }

    #[test]
    fn no_grant_within_two_seconds_fails_visibly_and_never_opens() {
        let mut floor = Floor::new("laptop-b", true);
        floor.request(at(0));
        assert!(floor.tick(at(1_900)).is_empty(), "too early");
        let timed_out = floor.tick(at(2_000));
        assert!(timed_out.iter().any(|a| matches!(a, Action::Refused(_))));
        assert!(!timed_out.contains(&Action::BeginTurn));
        assert_eq!(floor.holder(), Holder::Free);

        // The grant turns up late. The other side thinks we have the floor;
        // it is handed back, and the microphone still does not open.
        let late = floor.receive(&Wire::FloorGrant { seq: 1 });
        assert!(!late.contains(&Action::BeginTurn));
        assert_eq!(sent(&late), vec![Wire::FloorRelease { seq: 1 }]);
    }

    #[test]
    fn a_request_is_granted_and_the_floor_is_theirs_until_released() {
        let mut floor = Floor::new("laptop-a", false);
        let answered = floor.receive(&Wire::FloorRequest { seq: 7 });
        assert_eq!(sent(&answered), vec![Wire::FloorGrant { seq: 7 }]);
        assert_eq!(floor.holder(), Holder::Them("laptop-a".into()));

        // Pressing the key now is refused locally, with their name.
        let refused = floor.request(at(0));
        assert!(sent(&refused).is_empty());
        assert!(matches!(&refused[0], Action::Refused(why) if why.contains("laptop-a")));

        floor.receive(&Wire::FloorRelease { seq: 7 });
        assert_eq!(floor.holder(), Holder::Free);
    }

    #[test]
    fn simultaneous_requests_go_to_the_name_that_sorts_first() {
        // Both press at once. Each has sent its request and receives the
        // other's.
        let mut a = Floor::new("laptop-b", wins_ties("laptop-a", "laptop-b", "", ""));
        let mut b = Floor::new("laptop-a", wins_ties("laptop-b", "laptop-a", "", ""));
        let a_asks = sent(&a.request(at(0)));
        let b_asks = sent(&b.request(at(0)));

        let a_hears = a.receive(&b_asks[0]);
        let b_hears = b.receive(&a_asks[0]);

        assert!(a_hears.is_empty(), "the winner ignores the other's request");
        assert_eq!(sent(&b_hears), vec![Wire::FloorGrant { seq: 1 }]);
        assert!(
            b_hears.iter().any(|x| matches!(x, Action::Refused(_))),
            "the loser is told the other party has the floor"
        );
        assert_eq!(b.holder(), Holder::Them("laptop-a".into()));

        let a_granted = a.receive(&sent(&b_hears)[0]);
        assert!(a_granted.contains(&Action::BeginTurn));
        assert_eq!(a.holder(), Holder::Me);
    }

    #[test]
    fn a_turn_cancelled_before_the_grant_hands_the_floor_straight_back() {
        let mut floor = Floor::new("laptop-b", true);
        floor.request(at(0));
        floor.cancel();
        let granted = floor.receive(&Wire::FloorGrant { seq: 1 });
        assert!(!granted.contains(&Action::BeginTurn));
        assert_eq!(sent(&granted), vec![Wire::FloorRelease { seq: 1 }]);
        assert_eq!(floor.holder(), Holder::Free);
    }

    #[test]
    fn a_dropped_connection_force_releases_the_floor() {
        let mut floor = Floor::new("laptop-b", true);
        floor.request(at(0));
        floor.receive(&Wire::FloorGrant { seq: 1 });
        let dropped = floor.disconnected();
        assert!(
            dropped.contains(&Action::EndTurn),
            "the microphone stays open"
        );
        assert_eq!(floor.holder(), Holder::Free);

        let mut theirs = Floor::new("laptop-a", false);
        theirs.receive(&Wire::FloorRequest { seq: 1 });
        theirs.disconnected();
        assert_eq!(theirs.holder(), Holder::Free);
    }

    #[test]
    fn a_request_while_talking_gets_no_answer() {
        let mut floor = Floor::new("laptop-b", false);
        floor.request(at(0));
        floor.receive(&Wire::FloorGrant { seq: 1 });
        assert!(floor.receive(&Wire::FloorRequest { seq: 4 }).is_empty());
        assert_eq!(floor.holder(), Holder::Me);
    }

    #[test]
    fn ties_between_equal_names_are_still_decided_the_same_way_on_both_ends() {
        let a = wins_ties("pc", "pc", "10.0.0.1:50000", "10.0.0.2:47800");
        let b = wins_ties("pc", "pc", "10.0.0.2:47800", "10.0.0.1:50000");
        assert_ne!(a, b);
    }
}
