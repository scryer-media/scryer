//! The one place a placement policy is built.
//!
//! The disk service reads no settings: every [`Disk::place`](super::Disk::place)
//! request carries the whole policy, and the functions here are the only
//! callers that put one together. A caller names what it is doing and gets
//! the policy for it.

use super::{Depth, Evidence, Fallback, Landing, Method, Parents, Placement, Replace, Verify};
use crate::location::model::VerificationDepth;
use crate::location::verify::DestinationClaim;

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

/// Giving a file a second name that must prove to be that file, asked
/// through [`Disk::place_proven`](super::Disk::place_proven). A name already
/// taken is refused, and so is a link the volume cannot hold or one that
/// does not prove: the caller has its own verified copy for those.
pub fn proven_link() -> Placement {
    Placement {
        method: Method::HardLink,
        fallback: Fallback::None,
        replace: Replace::Never,
        verify: Verify::None,
        parents: Parents::MustExist,
    }
}

/// Copying a file and recording what the copy streamed and proved, at the
/// operation's configured verification depth. The source is left alone and
/// nothing is replaced: a copy that claims its own name is written beside it
/// and takes the name once complete; one whose name the caller already holds
/// is written over that held file. Folders are not created.
pub fn verified_copy(depth: VerificationDepth, claim: DestinationClaim) -> Placement {
    crate::location::transfer::install_copy_workers();
    recorded(crate::location::verify::disk_depth(depth), claim)
}

/// Copying into a staging name the caller journaled before asking. The name
/// is created by the copy, which fails if it is held, and whatever is written
/// stays there on failure for the caller's recovery. The caller proves the
/// copy from the hashes.
pub(crate) fn journaled_staging_copy() -> Placement {
    crate::location::transfer::install_copy_workers();
    Placement {
        verify: Verify::Recorded(Evidence {
            depth: Depth::Unproven,
            landing: Landing::Direct,
        }),
        ..recorded(Depth::Unproven, DestinationClaim::ClaimHere)
    }
}

/// A recorded copy the test proves for itself.
#[cfg(test)]
pub(crate) fn unproven_copy(claim: DestinationClaim) -> Placement {
    recorded(Depth::Unproven, claim)
}

fn recorded(depth: Depth, claim: DestinationClaim) -> Placement {
    let (replace, landing) = match claim {
        DestinationClaim::ClaimHere => (Replace::Never, Landing::Staged),
        DestinationClaim::AlreadyHeld => (Replace::Allowed, Landing::Direct),
    };
    Placement {
        method: Method::Copy,
        fallback: Fallback::None,
        replace,
        verify: Verify::Recorded(Evidence { depth, landing }),
        parents: Parents::MustExist,
    }
}
