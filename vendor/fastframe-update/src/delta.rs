//! MyAIRAW file-level incremental updates. Only publisher-authorized archives
//! are read; reconstruction happens in a new private staging directory. The
//! normal bundle validation, install helper and rollback remain authoritative.
use crate::{
    detect::Installation,
    download::Inputs,
    release::{self, Release},
    signing,
    stage::{self, Prepared, Staged},
    transport::fetch,
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path},
    process::Command,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    from: String,
    to: String,
    files: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    sha256: String,
    size: u64,
    mode: u32,
    #[serde(default)]
    payload: bool,
    #[serde(default)]
    link: Option<String>,
}

fn relative(path: &str) -> Result<&Path> {
    let p = Path::new(path);
    ensure!(
        !path.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_))),
        "Unsafe delta path"
    );
    Ok(p)
}

fn reconstruct(
    host: &dyn crate::host::Host,
    archive: &Path,
    old: &Path,
    destination: &Path,
    from: &str,
    to: &str,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let manifest_path = archive.with_extension("json");
    host.extract(archive, "manifest.json", &manifest_path)?;
    ensure!(
        fs::metadata(&manifest_path)?.len() <= release::MANIFEST_LIMIT,
        "Delta manifest too large"
    );
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    ensure!(
        manifest.schema == 1 && manifest.from == from && manifest.to == to,
        "Delta version mismatch"
    );
    ensure!(
        !manifest.files.is_empty() && manifest.files.len() <= 10000,
        "Invalid delta inventory"
    );
    fs::create_dir(destination)?;
    let mut paths = std::collections::HashSet::new();
    let mut total = 0u64;
    for entry in &manifest.files {
        let path = relative(&entry.path)?;
        ensure!(paths.insert(path), "Duplicate delta path");
        ensure!(
            entry.mode & !0o777 == 0 && entry.sha256.len() == 64,
            "Invalid delta file metadata"
        );
        total = total.checked_add(entry.size).context("Delta too large")?;
        ensure!(total <= stage::LIMIT, "Delta too large");
        // No entry may have another entry as an ancestor: files and links are leaves.
        ensure!(
            !manifest
                .files
                .iter()
                .any(|e| Path::new(&e.path) != path && path.starts_with(&e.path)),
            "Delta path traverses a file"
        );
        if let Some(link) = &entry.link {
            ensure!(
                !entry.payload && !Path::new(link).is_absolute(),
                "Unsafe delta symlink"
            );
            let mut depth = path.parent().map_or(0, |p| p.components().count());
            for component in Path::new(link).components() {
                match component {
                    Component::Normal(_) => depth += 1,
                    Component::CurDir => {}
                    Component::ParentDir if depth > 0 => depth -= 1,
                    _ => anyhow::bail!("Delta symlink escapes the bundle"),
                }
            }
        }
    }
    // All paths are checked before writing anything within the new bundle.
    for entry in &manifest.files {
        let path = relative(&entry.path)?;
        let target = destination.join(path);
        fs::create_dir_all(target.parent().context("Missing delta parent")?)?;
        if let Some(link) = &entry.link {
            use sha2::{Digest, Sha256};
            ensure!(
                crate::hex(&Sha256::digest(link.as_bytes())) == entry.sha256,
                "Delta link checksum mismatch"
            );
            std::os::unix::fs::symlink(link, &target)?;
        } else {
            if entry.payload {
                host.extract(archive, &format!("files/{}", entry.path), &target)?;
            } else {
                let source = old.join(path);
                ensure!(
                    !fs::symlink_metadata(&source)?.file_type().is_symlink(),
                    "Delta base is a link"
                );
                ensure!(
                    source.canonicalize()?.starts_with(old.canonicalize()?),
                    "Delta base escapes bundle"
                );
                fs::copy(source, &target)?;
            }
            ensure!(
                fs::metadata(&target)?.len() == entry.size && stage::hash(&target)? == entry.sha256,
                "Delta file checksum mismatch"
            );
            fs::set_permissions(&target, fs::Permissions::from_mode(entry.mode))?;
        }
    }
    Ok(())
}

pub(crate) fn download(
    inputs: &Inputs<'_>,
    installation: &Installation,
    release: &Release,
    progress: &mut impl FnMut(u64, u64),
) -> Result<Option<Prepared>> {
    let config = inputs.config;
    ensure!(
        config.accepts_version(&release.version),
        "Invalid delta release version"
    );
    // Never take an unsigned delta, even if a caller has allowed unsigned full packages.
    let key = config
        .publisher_key
        .context("Delta needs a publisher key")?;
    let target = release::target(inputs.platform, inputs.arch, config.mac_target)?;
    let metadata = release::metadata(config, inputs.transport, inputs.source, &release.version)?;
    let stem = release::stem(config, &release.version, target);
    let name = format!("{stem}-from-{}.delta.zip", config.current_version);
    let Ok(package) = metadata.asset(&name) else {
        return Ok(None);
    };
    let full = metadata.asset(&format!("{stem}.dmg"))?;
    if package.size >= full.size {
        return Ok(None);
    }
    let checksums = metadata.asset("checksums.txt")?;
    let signature = metadata.asset("checksums.txt.sig")?;
    ensure!(
        checksums.size <= release::MANIFEST_LIMIT && signature.size == 64,
        "Invalid delta signature metadata"
    );
    for asset in [package, checksums, signature] {
        ensure!(
            inputs.source.owns_asset(
                config,
                &url::Url::parse(&asset.browser_download_url)?,
                &release.version,
                &asset.name
            ),
            "Delta outside release"
        );
    }
    let mut manifest = Vec::new();
    fetch(
        inputs.transport,
        inputs.source,
        config,
        &checksums.browser_download_url,
        "application/octet-stream",
    )?
    .take(release::MANIFEST_LIMIT + 1)
    .read_to_end(&mut manifest)?;
    ensure!(
        manifest.len() as u64 == checksums.size,
        "Delta checksum manifest interrupted"
    );
    let mut signed = Vec::new();
    fetch(
        inputs.transport,
        inputs.source,
        config,
        &signature.browser_download_url,
        "application/octet-stream",
    )?
    .take(65)
    .read_to_end(&mut signed)?;
    let mut verified = signing::verify(&manifest, &signed, &signing::decode_key(key)?);
    for key in config.additional_publisher_keys {
        if verified.is_ok() {
            break;
        }
        verified = signing::verify(&manifest, &signed, &signing::decode_key(key)?);
    }
    verified?;
    let expected = release::checksum(std::str::from_utf8(&manifest)?, &name)?;
    let directory = stage::staging(config, installation)?;
    let result = (|| -> Result<Prepared> {
        let archive = directory.join(&name);
        let mut output = fs::File::create(&archive)?;
        let mut body = fetch(
            inputs.transport,
            inputs.source,
            config,
            &package.browser_download_url,
            "application/octet-stream",
        )?;
        let mut buffer = [0u8; 65536];
        let mut received = 0u64;
        loop {
            let count = body.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            received += count as u64;
            ensure!(received <= package.size, "Delta exceeds published size");
            output.write_all(&buffer[..count])?;
            progress(received, package.size);
        }
        output.sync_all()?;
        drop(output);
        ensure!(
            received == package.size && stage::hash(&archive)? == expected,
            "Delta download checksum mismatch"
        );
        let old = crate::macos::bundle_root(config, &installation.executable)
            .context("Missing installed bundle")?;
        ensure!(
            inputs.host.plist(old, "CFBundleShortVersionString")? == config.current_version,
            "Delta base version mismatch"
        );
        let payload_folder = directory.join("reconstructed");
        fs::create_dir(&payload_folder)?;
        let bundle = payload_folder.join(format!("{}.app", config.app_name));
        reconstruct(
            inputs.host,
            &archive,
            old,
            &bundle,
            config.current_version,
            &release.version,
        )?;
        let image = directory.join("reconstructed.dmg");
        let status = Command::new("/usr/bin/hdiutil")
            .args([
                "create",
                "-quiet",
                "-fs",
                "HFS+",
                "-format",
                "UDZO",
                "-srcfolder",
            ])
            .arg(&payload_folder)
            .arg(&image)
            .status()?;
        ensure!(status.success(), "Cannot stage delta disk image");
        crate::macos::validate_download(
            config,
            inputs.host,
            &image,
            installation,
            &release.version,
        )?;
        Ok(Prepared {
            staged: Staged {
                sha256: stage::hash(&image)?,
                installation: installation.clone(),
                directory: directory.clone(),
                payload: image,
                version: release.version.clone(),
            },
            sample: false,
        })
    })();
    match result {
        Ok(prepared) => Ok(Some(prepared)),
        Err(error) => {
            stage::discard(&directory);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_cannot_escape_the_new_bundle() {
        for path in ["", "../x", "/tmp/x", "a/../x", "./x"] {
            assert!(relative(path).is_err(), "{path}");
        }
        assert!(relative("Contents/MacOS/rawmakase").is_ok());
    }

    #[test]
    fn reconstruction_verifies_base_payload_and_paths() -> Result<()> {
        use sha2::{Digest, Sha256};
        let dir = tempfile::tempdir()?;
        let old = dir.path().join("old.app");
        fs::create_dir_all(old.join("Contents"))?;
        fs::write(old.join("Contents/keep"), b"kept")?;
        fs::create_dir_all(dir.path().join("files/Contents"))?;
        fs::write(dir.path().join("files/Contents/new"), b"new")?;
        let entries = serde_json::json!([
            {"path":"Contents/keep", "sha256":crate::hex(&Sha256::digest(b"kept")), "size":4, "mode":420, "payload":false},
            {"path":"Contents/new", "sha256":crate::hex(&Sha256::digest(b"new")), "size":3, "mode":493, "payload":true}
        ]);
        let make = |name: &str, files: serde_json::Value| -> Result<std::path::PathBuf> {
            fs::write(
                dir.path().join("manifest.json"),
                serde_json::to_vec(
                    &serde_json::json!({"schema":1,"from":"0.2.2","to":"0.2.3","files":files}),
                )?,
            )?;
            let archive = dir.path().join(format!("{name}.zip"));
            ensure!(
                Command::new("/usr/bin/zip")
                    .args(["-q", "-r"])
                    .arg(&archive)
                    .args(["manifest.json", "files"])
                    .current_dir(dir.path())
                    .status()?
                    .success(),
                "Cannot make test archive"
            );
            Ok(archive)
        };
        let archive = make("good", entries.clone())?;
        let new = dir.path().join("new.app");
        reconstruct(&crate::host::OsHost, &archive, &old, &new, "0.2.2", "0.2.3")?;
        assert_eq!(fs::read(new.join("Contents/keep"))?, b"kept");
        assert_eq!(fs::read(new.join("Contents/new"))?, b"new");
        fs::write(old.join("Contents/keep"), b"evil")?;
        let archive = make("corrupt-base", entries.clone())?;
        assert!(
            reconstruct(
                &crate::host::OsHost,
                &archive,
                &old,
                &dir.path().join("bad.app"),
                "0.2.2",
                "0.2.3"
            )
            .is_err()
        );
        let mut unsafe_entries = entries;
        unsafe_entries[0]["path"] = serde_json::json!("../escaped");
        let archive = make("unsafe", unsafe_entries)?;
        assert!(
            reconstruct(
                &crate::host::OsHost,
                &archive,
                &old,
                &dir.path().join("unsafe.app"),
                "0.2.2",
                "0.2.3"
            )
            .is_err()
        );
        assert!(!dir.path().join("escaped").exists());
        Ok(())
    }
}
