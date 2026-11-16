//! A connection kept between polls, e.g. to the Tesla gateway, so that each poll is a few requests rather than a new
//! HTTPS connection, which the polls which need the most RAM, those over the internet to myenergi and Octopus, close
//! first, see release, as the device runs its polls one at a time, and has no room for both.

use std::cell::RefCell;
use std::rc::Rc;

use bruce_esp_idf::svc::http::client::EspHttpConnection;
use embedded_svc::http::client::Client;

/// Shared by the poll which keeps it and those which release it, all of which run on the scheduler's thread.
#[derive(Clone, Default)]
pub struct KeptConnection(Rc<RefCell<Option<Client<EspHttpConnection>>>>);

impl KeptConnection {
    /// The connection, if there is one, for a poll, which puts it back with keep.
    pub fn take(&self) -> Option<Client<EspHttpConnection>> {
        self.0.borrow_mut().take()
    }

    pub fn keep(&self, client: Client<EspHttpConnection>) {
        *self.0.borrow_mut() = Some(client);
    }

    /// Closes the connection, if there is one, for a poll which needs the RAM, so that the next poll which uses it
    /// connects again.
    pub fn release(&self) {
        if self.0.borrow_mut().take().is_some() {
            log::debug!("Closed a kept connection, for the RAM");
        }
    }
}
