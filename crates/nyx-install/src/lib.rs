//! Installing NyxOS, without a UI: the disks there are ([`disks`]), what to install where
//! ([`plan`]) and carrying it out with progress ([`run`]). The installer app and the
//! `nyx-install` command both use it.

pub mod disks;
pub mod plan;
pub mod run;
