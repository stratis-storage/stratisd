// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    ffi::CString,
    fs::{create_dir_all, remove_file, set_permissions, OpenOptions, Permissions},
    io::{self, Read, Write},
    mem::size_of,
    os::unix::{
        fs::PermissionsExt,
        io::{AsRawFd, RawFd},
    },
    path::{Path, PathBuf},
    ptr, slice, str,
};

use libc::{syscall, SYS_add_key, SYS_keyctl};
use nix::{
    mount::{mount, umount, MsFlags},
    sched::{unshare, CloneFlags},
    sys::{
        mman::{mmap, munmap, MapFlags, ProtFlags},
        stat::stat,
    },
    unistd::{chown, gettid, Uid},
};
use rand::{
    distributions::{Alphanumeric, Distribution},
    thread_rng,
};

use libcryptsetup_rs::{SafeBorrowedMemZero, SafeMemHandle};

use crate::{
    engine::{
        engine::{KeyActions, MAX_STRATIS_PASS_SIZE},
        shared,
        strat_engine::names::KeyDescription,
        types::{Key, MappingCreateAction, MappingDeleteAction, SizedKeyMemory},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const INIT_MNT_NS_PATH: &str = "/proc/1/ns/mnt";

/// A type corresponding to key IDs in the kernel keyring. In `libkeyutils`,
/// this is represented as the C type `key_serial_t`.
type KeySerial = u32;

/// Search the persistent keyring for the given key description.
pub(super) fn search_key_persistent(key_desc: &KeyDescription) -> StratisResult<Option<KeySerial>> {
    let keyring_id = get_persistent_keyring()?;
    search_key(keyring_id, key_desc)
}

/// Read a key from the persistent keyring with the given key description.
pub(super) fn read_key_persistent(
    key_desc: &KeyDescription,
) -> StratisResult<Option<(KeySerial, SizedKeyMemory)>> {
    let keyring_id = get_persistent_keyring()?;
    read_key(keyring_id, key_desc)
}

/// Get the ID of the persistent root user keyring and attach it to
/// the session keyring.
fn get_persistent_keyring() -> StratisResult<KeySerial> {
    // Attach persistent keyring to session keyring
    match unsafe {
        syscall(
            SYS_keyctl,
            libc::KEYCTL_GET_PERSISTENT,
            0,
            libc::KEY_SPEC_SESSION_KEYRING,
        )
    } {
        i if i < 0 => Err(io::Error::last_os_error().into()),
        i => convert_int!(i, libc::c_long, KeySerial),
    }
}

/// Search for the given key description in the persistent root keyring.
/// Returns the key ID or nothing if it was not found in the keyring.
fn search_key(
    keyring_id: KeySerial,
    key_desc: &KeyDescription,
) -> StratisResult<Option<KeySerial>> {
    let key_desc_cstring = CString::new(key_desc.to_system_string()).map_err(|_| {
        StratisError::Engine(
            ErrorEnum::Invalid,
            "Invalid key description provided".to_string(),
        )
    })?;

    let key_id = unsafe {
        syscall(
            SYS_keyctl,
            libc::KEYCTL_SEARCH,
            keyring_id,
            concat!("user", "\0").as_ptr(),
            key_desc_cstring.as_ptr(),
            0,
        )
    };
    if key_id < 0 {
        if unsafe { *libc::__errno_location() } == libc::ENOKEY {
            Ok(None)
        } else {
            Err(io::Error::last_os_error().into())
        }
    } else {
        convert_int!(key_id, libc::c_long, KeySerial).map(Some)
    }
}

/// Read a key with the provided key description into safely handled memory if it
/// exists in the keyring.
///
/// The return type will be a tuple of an `Option` and a keyring id. The `Option`
/// type will be `Some` if the key was found in the keyring and will contain
/// the key ID and the key contents. If no key was found with the provided
/// key description, `None` will be returned.
fn read_key(
    keyring_id: KeySerial,
    key_desc: &KeyDescription,
) -> StratisResult<Option<(KeySerial, SizedKeyMemory)>> {
    let key_id_option = search_key(keyring_id, key_desc)?;
    let key_id = if let Some(ki) = key_id_option {
        ki
    } else {
        return Ok(None);
    };

    let mut key_buffer = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
    let mut_ref = key_buffer.as_mut();

    // Read key from keyring
    match unsafe {
        syscall(
            SYS_keyctl,
            libc::KEYCTL_READ,
            key_id,
            mut_ref.as_mut_ptr(),
            mut_ref.len(),
        )
    } {
        i if i < 0 => Err(io::Error::last_os_error().into()),
        i => Ok(Some((
            key_id as KeySerial,
            SizedKeyMemory::new(key_buffer, convert_int!(i, libc::c_long, usize)?),
        ))),
    }
}

/// Reset the key data attached to the provided key description if the new key data
/// is different from the old key data.
// Precondition: The key description is already present in the keyring.
fn reset_key(
    key_id: KeySerial,
    old_key_data: SizedKeyMemory,
    new_key_data: SizedKeyMemory,
) -> StratisResult<bool> {
    if old_key_data.as_ref() == new_key_data.as_ref() {
        Ok(false)
    } else {
        // Update the existing key data
        let update_result = unsafe {
            syscall(
                SYS_keyctl,
                libc::KEYCTL_UPDATE,
                key_id,
                new_key_data.as_ref().as_ptr(),
                new_key_data.as_ref().len(),
            )
        };
        if update_result < 0 {
            Err(io::Error::last_os_error().into())
        } else {
            Ok(true)
        }
    }
}

/// Add the key to the given keyring attaching it to the provided key description.
// Precondition: The key description was not already present.
fn set_key(
    key_desc: &KeyDescription,
    key_data: SizedKeyMemory,
    keyring_id: KeySerial,
) -> StratisResult<()> {
    let key_desc_cstring = CString::new(key_desc.to_system_string()).map_err(|_| {
        StratisError::Engine(
            ErrorEnum::Invalid,
            "Invalid key description provided".to_string(),
        )
    })?;
    // Add a key to the kernel keyring
    if unsafe {
        libc::syscall(
            SYS_add_key,
            concat!("user", "\0").as_ptr(),
            key_desc_cstring.as_ptr(),
            key_data.as_ref().as_ptr(),
            key_data.as_ref().len(),
            keyring_id,
        )
    } < 0
    {
        Err(io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}

/// Perform an idempotent add of the given key data with the given key description.
///
/// The unit type is returned as the inner type for `MappingCreateAction` as no
/// new external data (like a UUID) can be returned when setting a key. Keys
/// are identified by their key descriptions only unlike resources like pools
/// that have a name and a UUID.
///
/// Successful return values:
/// * `Ok(MappingCreateAction::Identity)`: The key was already in the keyring with the
/// appropriate key description and key data.
/// * `Ok(MappingCreateAction::Created(()))`: The key was newly added to the keyring.
/// * `Ok(MappingCreateAction::ValueChanged(()))`: The key description was already present
/// in the keyring but the key data was updated.
fn set_key_idem(
    key_desc: &KeyDescription,
    key_data: SizedKeyMemory,
) -> StratisResult<MappingCreateAction<Key>> {
    let keyring_id = get_persistent_keyring()?;
    match read_key(keyring_id, key_desc) {
        Ok(Some((key_id, old_key_data))) => {
            let changed = reset_key(key_id, old_key_data, key_data)?;
            if changed {
                Ok(MappingCreateAction::ValueChanged(Key))
            } else {
                Ok(MappingCreateAction::Identity)
            }
        }
        Ok(None) => {
            set_key(key_desc, key_data, keyring_id)?;
            Ok(MappingCreateAction::Created(Key))
        }
        Err(e) => Err(e),
    }
}

/// Parse the returned key string from `KEYCTL_DESCRIBE` into a key description.
fn parse_keyctl_describe_string(key_str: &str) -> StratisResult<String> {
    key_str
        .rsplit(';')
        .next()
        .map(|s| s.to_string())
        .ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::Invalid,
                "Invalid format returned from the kernel query for the key description".to_string(),
            )
        })
}

/// A list of key IDs that were read from the persistent root keyring.
struct KeyIdList {
    key_ids: Vec<KeySerial>,
}

impl KeyIdList {
    /// Create a new list of key IDs, with initial capacity of 4096
    fn new() -> KeyIdList {
        KeyIdList {
            key_ids: Vec::with_capacity(4096),
        }
    }

    /// Populate the list with IDs from the persistent root kernel keyring.
    fn populate(&mut self) -> StratisResult<()> {
        let keyring_id = get_persistent_keyring()?;

        // Read list of keys in the persistent keyring.
        let mut done = false;
        while !done {
            let num_bytes_read = match unsafe {
                syscall(
                    SYS_keyctl,
                    libc::KEYCTL_READ,
                    keyring_id,
                    self.key_ids.as_mut_ptr(),
                    self.key_ids.capacity(),
                )
            } {
                i if i < 0 => return Err(io::Error::last_os_error().into()),
                i => convert_int!(i, libc::c_long, usize)?,
            };

            let num_key_ids = num_bytes_read / size_of::<KeySerial>();

            if num_key_ids <= self.key_ids.capacity() {
                unsafe {
                    self.key_ids.set_len(num_key_ids);
                }
                done = true;
            } else {
                self.key_ids.resize(num_key_ids, 0);
            }
        }

        Ok(())
    }

    /// Get the list of key descriptions corresponding to the kernel key IDs.
    /// Return the subset of key descriptions that have a prefix that identify
    /// them as belonging to Stratis.
    fn to_key_descs(&self) -> StratisResult<Vec<KeyDescription>> {
        let mut key_descs = Vec::new();

        for id in self.key_ids.iter() {
            let mut keyctl_buffer: Vec<u8> = Vec::with_capacity(4096);

            let mut done = false;
            while !done {
                let len = match unsafe {
                    syscall(
                        SYS_keyctl,
                        libc::KEYCTL_DESCRIBE,
                        *id,
                        keyctl_buffer.as_mut_ptr(),
                        keyctl_buffer.capacity(),
                    )
                } {
                    i if i < 0 => return Err(io::Error::last_os_error().into()),
                    i => convert_int!(i, libc::c_long, usize)?,
                };

                if len <= keyctl_buffer.capacity() {
                    unsafe {
                        keyctl_buffer.set_len(len);
                    }
                    done = true;
                } else {
                    keyctl_buffer.resize(len, 0);
                }
            }

            if keyctl_buffer.is_empty() {
                return Err(StratisError::Error(format!(
                    "Kernel key description for key {} appeared to be entirely empty",
                    id
                )));
            }

            let keyctl_str =
                str::from_utf8(&keyctl_buffer[..keyctl_buffer.len() - 1]).map_err(|e| {
                    StratisError::Engine(
                        ErrorEnum::Invalid,
                        format!("Kernel key description was not valid UTF8: {}", e),
                    )
                })?;
            let parsed_string = parse_keyctl_describe_string(keyctl_str)?;
            if let Some(kd) = KeyDescription::from_system_key_desc(&parsed_string).map(|k| k.expect("parse_keyctl_desribe_string() ensures the key description can not have semi-colons in it")) {
                key_descs.push(kd);
            }
        }
        Ok(key_descs)
    }
}

/// Unset the key with ID `key_id` in the root peristent keyring.
fn unset_key(key_id: KeySerial) -> StratisResult<()> {
    let keyring_id = get_persistent_keyring()?;

    match unsafe { syscall(SYS_keyctl, libc::KEYCTL_UNLINK, key_id, keyring_id) } {
        i if i < 0 => Err(io::Error::last_os_error().into()),
        _ => Ok(()),
    }
}

/// Handle for kernel keyring interaction.
#[derive(Debug)]
pub struct StratKeyActions;

#[cfg(test)]
impl StratKeyActions {
    /// Method used in testing to bypass the need to provide a file descriptor
    /// when setting the key. This method allows passing memory to the engine API
    /// for adding keys and removes the need for a backing file or interactive entry
    /// of the key. This method is only useful for testing stratisd internally. It
    /// is not useful for testing using D-Bus.
    pub fn set_no_fd(
        &mut self,
        key_desc: &KeyDescription,
        key: SizedKeyMemory,
    ) -> StratisResult<MappingCreateAction<Key>> {
        Ok(set_key_idem(&key_desc, key)?)
    }
}

impl KeyActions for StratKeyActions {
    fn set(
        &mut self,
        key_desc: &KeyDescription,
        key_fd: RawFd,
    ) -> StratisResult<MappingCreateAction<Key>> {
        let memory = shared::set_key_shared(key_fd)?;

        Ok(set_key_idem(key_desc, memory)?)
    }

    fn list(&self) -> StratisResult<Vec<KeyDescription>> {
        let mut key_ids = KeyIdList::new();
        key_ids.populate()?;
        key_ids.to_key_descs()
    }

    fn unset(&mut self, key_desc: &KeyDescription) -> StratisResult<MappingDeleteAction<Key>> {
        let keyring_id = get_persistent_keyring()?;

        if let Some(key_id) = search_key(keyring_id, key_desc)? {
            unset_key(key_id).map(|_| MappingDeleteAction::Deleted(Key))
        } else {
            Ok(MappingDeleteAction::Identity)
        }
    }
}

/// A top-level tmpfs that can be made a private recursive mount so that any tmpfs
/// mounts inside of it will not be visible to any process but stratisd.
#[derive(Debug)]
pub struct MemoryFilesystem;

impl MemoryFilesystem {
    pub const TMPFS_LOCATION: &'static str = "/run/stratisd/keyfiles";

    pub fn new() -> StratisResult<MemoryFilesystem> {
        let tmpfs_path = &Path::new(Self::TMPFS_LOCATION);
        if tmpfs_path.exists() {
            if !tmpfs_path.is_dir() {
                return Err(StratisError::Io(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a directory", tmpfs_path.display()),
                )));
            } else {
                let stat_info = stat(Self::TMPFS_LOCATION)?;
                let parent_path: PathBuf = vec![Self::TMPFS_LOCATION, ".."].iter().collect();
                let parent_stat_info = stat(&parent_path)?;
                if stat_info.st_dev != parent_stat_info.st_dev {
                    info!("Mount found at {}; unmounting", Self::TMPFS_LOCATION);
                    if let Err(e) = umount(Self::TMPFS_LOCATION) {
                        warn!(
                            "Failed to unmount filesystem at {}: {}",
                            Self::TMPFS_LOCATION,
                            e
                        );
                    }
                }
            }
        } else {
            create_dir_all(Self::TMPFS_LOCATION)?;
        };
        mount(
            Some("tmpfs"),
            Self::TMPFS_LOCATION,
            Some("tmpfs"),
            MsFlags::empty(),
            Some("size=1M"),
        )?;

        mount::<str, str, str, str>(
            None,
            MemoryFilesystem::TMPFS_LOCATION,
            None,
            MsFlags::MS_SLAVE | MsFlags::MS_REC,
            None,
        )?;
        Ok(MemoryFilesystem)
    }
}

impl Drop for MemoryFilesystem {
    fn drop(&mut self) {
        if let Err(e) = umount(Self::TMPFS_LOCATION) {
            warn!(
                "Could not unmount temporary in memory storage for Clevis keyfiles: {}",
                e
            );
        }
    }
}

/// Check if the stratisd mount namespace for this thread is in the root namespace.
fn is_in_root_namespace() -> StratisResult<bool> {
    let pid_one_stat = stat(INIT_MNT_NS_PATH)?;
    let self_stat = stat(format!("/proc/self/task/{}/ns/mnt", gettid()).as_str())?;
    Ok(pid_one_stat.st_ino == self_stat.st_ino)
}

/// An in-memory filesystem that mounts a tmpfs that can house keyfiles so that they
/// are never writen to disk. The interface aims to keep the keys in memory for as
/// short of a period of time as possible (only for the duration of the operation
/// that the keyfile is needed for).
pub struct MemoryPrivateFilesystem(PathBuf);

impl MemoryPrivateFilesystem {
    pub fn new() -> StratisResult<MemoryPrivateFilesystem> {
        let tmpfs_path = &Path::new(MemoryFilesystem::TMPFS_LOCATION);
        if tmpfs_path.exists() {
            if !tmpfs_path.is_dir() {
                return Err(StratisError::Io(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a directory", tmpfs_path.display()),
                )));
            } else {
                let stat_info = stat(MemoryFilesystem::TMPFS_LOCATION)?;
                let parent_path: PathBuf = vec![MemoryFilesystem::TMPFS_LOCATION, ".."]
                    .iter()
                    .collect();
                let parent_stat_info = stat(&parent_path)?;
                if stat_info.st_dev == parent_stat_info.st_dev {
                    return Err(StratisError::Io(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!(
                            "No mount found at {} which is required to proceed",
                            tmpfs_path.display(),
                        ),
                    )));
                }
            }
        } else {
            return Err(StratisError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Path {} does not exist", MemoryFilesystem::TMPFS_LOCATION,),
            )));
        };
        let random_string = Alphanumeric
            .sample_iter(thread_rng())
            .take(16)
            .collect::<String>();
        let private_fs_path = vec![MemoryFilesystem::TMPFS_LOCATION, &random_string]
            .iter()
            .collect();
        create_dir_all(&private_fs_path)?;

        // Only create a new mount namespace if the thread is in the root namespace.
        if is_in_root_namespace()? {
            unshare(CloneFlags::CLONE_NEWNS)?;
        }
        // Check that the namespace is now different.
        if is_in_root_namespace()? {
            return Err(StratisError::Error(
                "It was detected that the in-memory key files would have ended up \
                visible on the host system; aborting operation prior to generating \
                in memory key file"
                    .to_string(),
            ));
        }

        // Ensure that the original tmpfs mount point is private. This will work
        // even if someone mounts their own volume at this mount point as the
        // mount only needs to be private, it does not need to be tmpfs.
        // The mount directly after this one will also be a tmpfs meaning that
        // no keys will be written to disk even if this mount turns out to be
        // a physical device.
        mount::<str, str, str, str>(
            None,
            MemoryFilesystem::TMPFS_LOCATION,
            None,
            MsFlags::MS_SLAVE | MsFlags::MS_REC,
            None,
        )?;
        mount(
            Some("tmpfs"),
            &private_fs_path,
            Some("tmpfs"),
            MsFlags::empty(),
            Some("size=1M"),
        )?;
        Ok(MemoryPrivateFilesystem(private_fs_path))
    }

    pub fn key_op<F>(&self, key_desc: &KeyDescription, mut f: F) -> StratisResult<()>
    where
        F: FnMut(&Path) -> StratisResult<()>,
    {
        let persistent_id = get_persistent_keyring()?;
        let key_data = if let Some((_, mem)) = read_key(persistent_id, key_desc)? {
            mem
        } else {
            return Err(StratisError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "Key with given key description {} was not found",
                    key_desc.as_application_str()
                ),
            )));
        };
        let mut mem_file_path = PathBuf::from(&self.0);
        mem_file_path.push(key_desc.as_application_str());
        let mem_file = MemoryMappedKeyfile::new(&mem_file_path, key_data)?;
        f(mem_file.keyfile_path())
    }

    pub fn rand_key(&self) -> StratisResult<MemoryMappedKeyfile> {
        let mut key_data = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
        File::open("/dev/urandom")?.read_exact(key_data.as_mut())?;
        let mut mem_file_path = PathBuf::from(&self.0);
        mem_file_path.push(
            Alphanumeric
                .sample_iter(thread_rng())
                .take(10)
                .collect::<String>(),
        );
        MemoryMappedKeyfile::new(
            &mem_file_path,
            SizedKeyMemory::new(key_data, MAX_STRATIS_PASS_SIZE),
        )
    }
}

impl Drop for MemoryPrivateFilesystem {
    fn drop(&mut self) {
        if let Err(e) = umount(&self.0) {
            warn!(
                "Could not unmount temporary in memory storage for Clevis keyfiles: {}",
                e
            );
        }
    }
}

/// Keyfile integration with Clevis for keys so that they are never written to disk.
/// This struct will handle memory mapping and locking internally to avoid disk usage.
pub struct MemoryMappedKeyfile(*mut libc::c_void, usize, PathBuf);

impl MemoryMappedKeyfile {
    pub fn new(file_path: &Path, key_data: SizedKeyMemory) -> StratisResult<MemoryMappedKeyfile> {
        debug!(
            "Initializing in memory keyfile at path {}",
            file_path.display()
        );
        if file_path.exists() {
            return Err(StratisError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Keyfile is already present",
            )));
        }

        let keyfile = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(file_path)?;
        assert!(Uid::current().is_root());
        chown(file_path, Some(Uid::current()), None)?;
        set_permissions(file_path, Permissions::from_mode(0o600))?;
        let needed_keyfile_length = key_data.as_ref().len();
        keyfile.set_len(convert_int!(needed_keyfile_length, usize, u64)?)?;
        let mem = unsafe {
            mmap(
                ptr::null_mut(),
                needed_keyfile_length,
                ProtFlags::PROT_WRITE,
                MapFlags::MAP_SHARED | MapFlags::MAP_LOCKED,
                keyfile.as_raw_fd(),
                0,
            )
        }?;
        let mut slice = unsafe { slice::from_raw_parts_mut(mem as *mut u8, needed_keyfile_length) };
        slice.write_all(key_data.as_ref())?;
        Ok(MemoryMappedKeyfile(
            mem,
            needed_keyfile_length,
            file_path.to_owned(),
        ))
    }

    pub fn keyfile_path(&self) -> &Path {
        &self.2
    }
}

impl AsRef<[u8]> for MemoryMappedKeyfile {
    fn as_ref(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.0 as *const u8, self.1) }
    }
}

impl Drop for MemoryMappedKeyfile {
    fn drop(&mut self) {
        {
            unsafe { SafeBorrowedMemZero::from_ptr(self.0, self.1) };
        }
        if let Err(e) = unsafe { munmap(self.0, self.1) } {
            warn!("Could not unmap temporary keyfile: {}", e);
        }
        if let Err(e) = remove_file(self.keyfile_path()) {
            warn!("Failed to clean up temporary key file: {}", e);
        }
    }
}
