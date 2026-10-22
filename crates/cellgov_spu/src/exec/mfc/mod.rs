//! The MFC command path: what a write to MFC_Cmd or MFC_WrListStallAck
//! queues.

mod command;
mod list;
mod ordering;
mod transfer;

pub(super) use command::execute_mfc_cmd;
pub(crate) use command::{invalid_command, mfc_barrier_kind};
pub(super) use list::acknowledge_list_stall;
pub(crate) use list::start_held_lists;
