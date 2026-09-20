//! The debug observers a boot installs for the program driving it.

use std::collections::BTreeMap;
use std::rc::Rc;

use cellgov_core::RuntimeTap;
use cellgov_mem::GuestMemory;
use cellgov_ppu::PpuTap;

/// The debug observers a boot installs.
///
/// The boot gives the observer [`Self::ppu`] returns to every PPU unit
/// it creates:
///
/// - the title's primary,
/// - each `module_start` unit,
/// - each guest-created thread,
/// - each unit of a spawned child.
///
/// The boot asks for [`Self::runtime`] once, when the runtime exists.
pub trait DebugTaps {
    /// The observer every PPU unit reports its dispatches to.
    ///
    /// The boot calls this more than once. Every call returns the same
    /// observer.
    fn ppu(&self) -> Option<Rc<dyn PpuTap>> {
        None
    }

    /// The observer the runtime reports memory writes and steps to.
    fn runtime(&self) -> Option<Box<dyn RuntimeTap>> {
        None
    }

    /// The boot bound a firmware set into `mem`.
    ///
    /// `exports` maps each library to its NIDs, and each NID to the
    /// guest address of its OPD. The boot calls this for two sets:
    ///
    /// - its own set, once, before any `module_start` runs. `exports` is
    ///   empty when the boot loads no firmware set.
    /// - a spawned child's set, once the spawn loader can no longer
    ///   refuse the spawn. `mem` is the child's memory. A refused spawn
    ///   reports nothing, and a boot with no firmware directory reports
    ///   no child set.
    fn firmware_bound(
        &self,
        _space: u32,
        _exports: &BTreeMap<String, BTreeMap<u32, u32>>,
        _mem: &GuestMemory,
    ) {
    }
}

/// A [`DebugTaps`] that installs no observer.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTaps;

impl DebugTaps for NoTaps {}
