//! The one place a placement policy is built.
//!
//! The disk service reads no settings: every [`Disk::place`](super::Disk::place)
//! request carries the whole policy, and the functions here are the only
//! callers that put one together. A caller names what it is doing and gets
//! the policy for it.

use super::{Fallback, Method, Parents, Placement, Replace, Verify};

/// Renaming a library file to its planned name. Nothing at the new name is
/// ever replaced; a name on another volume is reached by a copy that is
/// proven before the old file goes; missing folders are created.
pub fn library_rename() -> Placement {
    Placement {
        method: Method::Move,
        fallback: Fallback::Copy,
        replace: Replace::Never,
        verify: Verify::Quick,
        parents: Parents::Create,
    }
}

/// Moving by rename alone onto a name the caller holds or is entitled to
/// replace: a recycle-bin slot, a restore the operator confirmed, a folder
/// it just reserved. A move the rename cannot make is refused, for a caller
/// with its own verified copy.
pub fn rename_onto_held_name() -> Placement {
    Placement {
        method: Method::Move,
        fallback: Fallback::None,
        replace: Replace::Allowed,
        verify: Verify::None,
        parents: Parents::MustExist,
    }
}

/// Giving a recycled file a second name in the library. A name already taken
/// is refused; a link the volume cannot hold is refused too, for the
/// recycle bin's own verified copy.
pub(crate) fn recycle_restore_link() -> Placement {
    Placement {
        method: Method::HardLink,
        fallback: Fallback::None,
        replace: Replace::Never,
        verify: Verify::None,
        parents: Parents::MustExist,
    }
}
