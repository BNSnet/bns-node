#![warn(missing_docs)]

//! All messages should be sent with `MessageRelay`.
//! By calling `relay` method in correct place, `MessageRelay` help to do things:
//! - Infer the next_hop of a message.
//! - Get the sender and origin sender of a message.
//! - Record the whole transport path for inspection.

use serde::Deserialize;
use serde::Serialize;

use crate::dht::Did;
use crate::err::Error;
use crate::err::Result;

/// `MessageRelay` divides messages into two types by method: SEND and REPORT.
/// And will enable different behaviors when handling SEND and REPORT messages.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum RelayMethod {
    /// When a node want to send message to another node, it will send a message with SEND method.
    SEND,
    /// A node that got a SEND message will either transpond it to another node or respond with a REPORT message.
    REPORT,
}

/// MessageRelay guide message passing on rings network by relay.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct MessageRelay {
    /// The method of message. SEND or REPORT.
    pub method: RelayMethod,

    /// A push only stack. Record routes when handling sending messages.
    pub path: Vec<Did>,

    /// Move this cursor to flag the top of the stack when reporting.
    /// Notice that this cursor is not the index of current.
    /// It's `path.len() - <index of current> - 1`, which means count down to head of vector.
    /// It will always be 0 while handling sending messages in this way.
    pub path_end_cursor: usize,

    /// The next node to handle the message.
    /// When and only when located at the end of the message propagation, the `next_hop` is none.
    /// The current handler will pick transport by this field.
    pub next_hop: Option<Did>,

    /// The destination of the message. It may be customized when sending. It cannot be changed when reporting.
    /// It may help the handler to find out `next_hop` in some situations.
    pub destination: Did,
}

impl MessageRelay {
    /// Create a new `MessageRelay`.
    /// Will set `path_end_cursor` to 0 if got None in parameter.
    pub fn new(
        method: RelayMethod,
        path: Vec<Did>,
        path_end_cursor: Option<usize>,
        next_hop: Option<Did>,
        destination: Did,
    ) -> Self {
        Self {
            method,
            path,
            path_end_cursor: path_end_cursor.unwrap_or(0),
            next_hop,
            destination,
        }
    }

    /// Check current did, update path and its end cursor, then infer next_hop.
    ///
    /// When handling a SEND message, will push `current` to the `self.path` stack, and set `next_hop` parameter to `self.next_node`.
    ///
    /// When handling a REPORT message, will move forward `self.path_end_cursor` to the position of `current` in `self.path`.
    /// If `next_hop` parameter is none, it will also pick the previous node in `self.path` as `self.next_hop`.
    /// (With this feature, one can always pass None as `next_hop` parameter when handling a REPORT message.)
    pub fn relay(&mut self, current: Did, next_hop: Option<Did>) -> Result<()> {
        self.validate()?;

        // If self.next_hop is setted, it should be current
        if self.next_hop.is_some() && self.next_hop.unwrap() != current {
            return Err(Error::InvalidNextHop);
        }

        match self.method {
            RelayMethod::SEND => {
                self.path.push(current);
                self.next_hop = next_hop;
                Ok(())
            }

            RelayMethod::REPORT => {
                // The final hop
                if self.next_hop == Some(self.destination) {
                    self.path_end_cursor = self.path.len() - 1;
                    self.next_hop = None;
                    return Ok(());
                }

                let pos = self
                    .path
                    .iter()
                    .rev()
                    .skip(self.path_end_cursor)
                    .position(|&x| x == current);

                if let (None, None) = (pos, next_hop) {
                    return Err(Error::CannotInferNextHop);
                }

                if let Some(pos) = pos {
                    self.path_end_cursor += pos;
                }

                // `self.path_prev()` should never return None here, because we have handled final hop
                self.next_hop = next_hop.or_else(|| self.path_prev());

                Ok(())
            }
        }
    }

    /// Construct an `MessageRelay` of method REPORT from a `MessageRelay` of method REPORT.
    /// It will return Error if method is not SEND.
    /// It will return Error if `self.path.len()` is less than 2.
    pub fn report(&self) -> Result<Self> {
        if self.method != RelayMethod::SEND {
            return Err(Error::ReportNeedSend);
        }

        if self.path.len() < 2 {
            return Err(Error::CannotInferNextHop);
        }

        Ok(Self {
            method: RelayMethod::REPORT,
            path: self.path.clone(),
            path_end_cursor: 0,
            next_hop: self.path_prev(),
            destination: self.sender(),
        })
    }

    /// A SEND message can change its destination.
    /// Call with REPORT method will get an error imeediately.
    pub fn reset_destination(&mut self, destination: Did) -> Result<()> {
        if self.method == RelayMethod::SEND {
            self.destination = destination;
            Ok(())
        } else {
            Err(Error::ResetDestinationNeedSend)
        }
    }

    /// Check if path and destination is valid.
    /// It will be automatically called at relay started.
    pub fn validate(&self) -> Result<()> {
        // Adjacent elements in self.path cannot be equal
        if self.path.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::InvalidRelayPath);
        }

        // The destination of report message should always be the first element of path
        if self.method == RelayMethod::REPORT && self.path[0] != self.destination {
            return Err(Error::InvalidRelayDestination);
        }

        Ok(())
    }

    /// Get the original sender of current message.
    /// Should always be the first element of path.
    pub fn origin(&self) -> Did {
        *self.path.first().unwrap()
    }

    /// Get sender of current message.
    /// With SEND method, it will be the `origin()` of the message.
    /// With REPORT method, it will be the last element of path.
    pub fn sender(&self) -> Did {
        match self.method {
            RelayMethod::SEND => self.origin(),
            RelayMethod::REPORT => *self.path.last().unwrap(),
        }
    }

    /// Get the previous element of the element pointed by path_end_cursor.
    pub fn path_prev(&self) -> Option<Did> {
        if self.path.len() < self.path_end_cursor + 2 {
            None
        } else {
            Some(self.path[self.path.len() - 2 - self.path_end_cursor])
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ecc::SecretKey;

    #[test]
    fn test_path_end_cursor() {
        let origin_sender = SecretKey::random().address().into();
        let next_hop1 = SecretKey::random().address().into();
        let next_hop2 = SecretKey::random().address().into();
        let next_hop3 = SecretKey::random().address().into();

        let mut send_relay = MessageRelay {
            method: RelayMethod::SEND,
            path: vec![origin_sender],
            path_end_cursor: 0,
            next_hop: None,
            destination: next_hop3,
        };

        // node0 -> node1
        send_relay.relay(next_hop1, None).unwrap();
        assert_eq!(send_relay.path_end_cursor, 0);

        // node0 -> node1 -> node2
        send_relay.relay(next_hop2, None).unwrap();
        assert_eq!(send_relay.path_end_cursor, 0);

        // node0 -> node1 -> node2 -> node3
        send_relay.relay(next_hop3, None).unwrap();
        assert_eq!(send_relay.path_end_cursor, 0);

        // node3 make REPORT, destination is node0
        let mut report_relay = send_relay.report().unwrap();
        assert_eq!(report_relay.path_end_cursor, 0);

        // node0 -> node1 -> node2 -> node3 -> node2
        report_relay.relay(next_hop2, None).unwrap();
        assert_eq!(report_relay.path_end_cursor, 1);

        // node0 -> node1 -> node2 -> node3 -> node2 -> node1
        report_relay.relay(next_hop1, None).unwrap();
        assert_eq!(report_relay.path_end_cursor, 2);
    }

    #[test]
    fn test_path_prev() {
        let origin_sender = SecretKey::random().address().into();
        let next_hop1 = SecretKey::random().address().into();
        let next_hop2 = SecretKey::random().address().into();

        let mut relay = MessageRelay {
            method: RelayMethod::SEND,
            path: vec![origin_sender],
            path_end_cursor: 0,
            next_hop: None,
            destination: next_hop2,
        };

        assert!(relay.path_prev().is_none());

        relay.relay(next_hop1, None).unwrap();
        assert_eq!(relay.path_prev(), Some(origin_sender));

        relay.relay(next_hop2, None).unwrap();
        assert_eq!(relay.path_prev(), Some(next_hop1));
    }
}
