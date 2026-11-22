//! The XMPP gateway: the bot as an external component (XEP-0114) of an
//! XMPP server on the same host, with a contact for each avatar, one for
//! local chat and one for commands.

pub mod address;
pub mod chat;
pub mod conn;
pub mod gateway;
pub mod presence;
pub mod queue;
pub mod stanza;
pub mod stream;
