//! Creating spaces, reading them, and tagging units with the space they run in.

use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_sync::ReservationTable;

use crate::runtime::state::Runtime;

use super::table::{AddressSpaceId, SpaceError};

impl Runtime {
    /// Create an empty child address space.
    ///
    /// [`Runtime::create_address_space_with`] takes the memory instead.
    ///
    /// # Errors
    /// [`SpaceError::SpaceExists`] for space 0 or a duplicate id.
    pub fn create_address_space(&mut self, space: AddressSpaceId) -> Result<(), SpaceError> {
        self.create_address_space_with(
            space,
            GuestMemory::from_regions(Vec::new()).expect("empty region set cannot overlap"),
        )
    }

    /// Create a child address space over `memory`.
    ///
    /// From here bytes reach the space through the commit pipeline or
    /// [`Runtime::place_bytes`].
    ///
    /// # Errors
    /// [`SpaceError::SpaceExists`] for space 0 or a duplicate id.
    pub fn create_address_space_with(
        &mut self,
        space: AddressSpaceId,
        memory: GuestMemory,
    ) -> Result<(), SpaceError> {
        if space == AddressSpaceId::BOOT || self.spaces.extra.contains_key(&space) {
            return Err(SpaceError::SpaceExists(space.raw()));
        }
        self.spaces.extra.insert(space, memory);
        self.spaces
            .extra_reservations
            .insert(space, ReservationTable::in_space(space.raw()));
        Ok(())
    }

    /// Every address space with its memory, space 0 first, then child
    /// spaces in id order.
    pub fn address_spaces(&self) -> impl Iterator<Item = (AddressSpaceId, &GuestMemory)> {
        std::iter::once((AddressSpaceId::BOOT, &self.memory))
            .chain(self.spaces.extra.iter().map(|(id, mem)| (*id, mem)))
    }

    /// Read view of `space`'s memory.
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when no such space exists.
    pub fn space_memory(&self, space: AddressSpaceId) -> Result<&GuestMemory, SpaceError> {
        if space == AddressSpaceId::BOOT {
            return Ok(&self.memory);
        }
        self.spaces
            .extra
            .get(&space)
            .ok_or(SpaceError::UnknownSpace(space.raw()))
    }

    /// Mutable view of `space`'s memory, for a test that shapes a
    /// space in place.
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when no such space exists.
    #[cfg(test)]
    pub(crate) fn space_memory_mut(
        &mut self,
        space: AddressSpaceId,
    ) -> Result<&mut GuestMemory, SpaceError> {
        if space == AddressSpaceId::BOOT {
            return Ok(&mut self.memory);
        }
        self.spaces
            .extra
            .get_mut(&space)
            .ok_or(SpaceError::UnknownSpace(space.raw()))
    }

    /// Read view of `space`'s reservation table.
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when no such space exists.
    pub fn space_reservations(
        &self,
        space: AddressSpaceId,
    ) -> Result<&ReservationTable, SpaceError> {
        if space == AddressSpaceId::BOOT {
            return Ok(&self.reservations);
        }
        self.spaces
            .extra_reservations
            .get(&space)
            .ok_or(SpaceError::UnknownSpace(space.raw()))
    }

    /// Mutable view of `space`'s reservation table, for a test that
    /// seeds a reservation.
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when no such space exists.
    #[cfg(test)]
    pub(crate) fn space_reservations_mut(
        &mut self,
        space: AddressSpaceId,
    ) -> Result<&mut ReservationTable, SpaceError> {
        if space == AddressSpaceId::BOOT {
            return Ok(&mut self.reservations);
        }
        self.spaces
            .extra_reservations
            .get_mut(&space)
            .ok_or(SpaceError::UnknownSpace(space.raw()))
    }

    /// Assign `unit` to `space`. Untagged units are space 0; tagging
    /// back to [`AddressSpaceId::BOOT`] removes the entry.
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when the space does not exist.
    pub fn assign_unit_space(
        &mut self,
        unit: UnitId,
        space: AddressSpaceId,
    ) -> Result<(), SpaceError> {
        if space == AddressSpaceId::BOOT {
            self.spaces.unit_spaces.remove(unit);
            return Ok(());
        }
        if !self.spaces.extra.contains_key(&space) {
            return Err(SpaceError::UnknownSpace(space.raw()));
        }
        self.spaces.unit_spaces.insert(unit, space);
        Ok(())
    }

    /// The space `unit` executes in.
    pub fn unit_space(&self, unit: UnitId) -> AddressSpaceId {
        self.spaces.space_of(unit)
    }
}
