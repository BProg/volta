use std::fmt::{self, Display};
use std::path::Path;

use super::Tool;
use crate::error::{Context, ErrorKind, Fallible};
use crate::fs::{create_staging_dir, remove_dir_if_exists, rename};
use crate::layout::volta_home;
use crate::platform::{Image, PlatformSpec};
use crate::session::Session;
use crate::style::{success_prefix, tool_version};
use crate::sync::VoltaLock;
use crate::version::VersionSpec;
use fs_utils::ensure_containing_dir_exists;
use log::info;
use tempfile::TempDir;

mod configure;
mod install;
mod manager;
mod metadata;
mod uninstall;

pub use manager::PackageManager;
pub use metadata::{BinConfig, PackageConfig, PackageManifest};
pub use uninstall::uninstall;

/// The Tool implementation for installing 3rd-party global packages
pub struct Package {
    name: String,
    version: VersionSpec,
    staging: TempDir,
}

impl Package {
    pub fn new(name: String, version: VersionSpec) -> Fallible<Self> {
        let staging = create_staging_dir()?;
        Ok(Package {
            name,
            version,
            staging,
        })
    }

    pub fn run_install(&self, platform_image: &Image) -> Fallible<()> {
        install::run_global_install(
            self.to_string(),
            self.staging.path().to_owned(),
            platform_image,
        )
    }

    pub fn complete_install(self, image: &Image) -> Fallible<PackageManifest> {
        let manager = PackageManager::Npm;
        let manifest =
            configure::parse_manifest(&self.name, self.staging.path().to_owned(), manager)?;

        persist_install(&self.name, &self.version, self.staging.path())?;
        configure::write_config_and_shims(&self.name, &manifest, image, manager)?;

        Ok(manifest)
    }
}

impl Tool for Package {
    fn fetch(self: Box<Self>, _session: &mut Session) -> Fallible<()> {
        Err(ErrorKind::CannotFetchPackage {
            package: self.to_string(),
        }
        .into())
    }

    fn install(self: Box<Self>, session: &mut Session) -> Fallible<()> {
        let _lock = VoltaLock::acquire()?;

        let default_image = session
            .default_platform()?
            .map(PlatformSpec::as_default)
            .ok_or(ErrorKind::NoPlatform)?
            .checkout(session)?;

        self.run_install(&default_image)?;
        let manifest = self.complete_install(&default_image)?;

        let bins = manifest.bin.join(", ");

        info!(
            "{} installed {} with executables: {}",
            success_prefix(),
            tool_version(manifest.name, manifest.version),
            bins
        );

        Ok(())
    }

    fn pin(self: Box<Self>, _session: &mut Session) -> Fallible<()> {
        Err(ErrorKind::CannotPinPackage { package: self.name }.into())
    }
}

impl Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.version {
            VersionSpec::None => f.write_str(&self.name),
            _ => f.write_str(&tool_version(&self.name, &self.version)),
        }
    }
}

fn persist_install(
    package_name: &str,
    package_version: &VersionSpec,
    staging_dir: &Path,
) -> Fallible<()> {
    let package_dir = volta_home()?.package_image_dir(package_name);

    remove_dir_if_exists(&package_dir)?;

    // Handle scoped packages (@vue/cli), which have an extra directory for the scope
    ensure_containing_dir_exists(&package_dir).with_context(|| ErrorKind::ContainingDirError {
        path: package_dir.to_owned(),
    })?;

    rename(staging_dir, &package_dir).with_context(|| ErrorKind::SetupToolImageError {
        tool: package_name.into(),
        version: package_version.to_string(),
        dir: package_dir,
    })?;

    Ok(())
}
