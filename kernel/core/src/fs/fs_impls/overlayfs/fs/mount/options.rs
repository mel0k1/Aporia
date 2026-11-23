#![short_vis_path::add(overlayfs)]

use crate::{
    fs::vfs::{file_system::FsFlags, xattr::XattrNamespace},
    prelude::*,
};

#[derive(Debug)]
pub(super) struct MountOptions {
    is_default_permissions: bool,
    pub(super) lower_dirs: Vec<String>,
    pub(super) upper_dir: Option<String>,
    pub(super) work_dir: Option<String>,
    pub(super) is_forced_read_only: bool,
    pub(super) xattr_namespace: XattrNamespace,
    pub(super) uuid_mode: Option<UuidMode>,
    pub(super) xino_mode: Option<XinoMode>,
    redirect_dir: Option<RedirectDirMode>,
    index: Option<bool>,
    nfs_export: Option<bool>,
    metacopy: Option<bool>,
    verity: Option<VerityMode>,
    fsync_mode: Option<FsyncMode>,
    is_lowerdir_plus_seen: bool,
    is_nooverride_creds_seen: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UuidMode {
    Off,
    Null,
    On,
    Auto,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) enum XinoMode {
    Off,
    Auto,
    On,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedirectDirMode {
    Off,
    Follow,
    NoFollow,
    On,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VerityMode {
    Off,
    On,
    Require,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FsyncMode {
    Volatile,
    Auto,
    Strict,
}

impl MountOptions {
    pub(super) fn parse(args: Option<&str>, fs_flags: FsFlags) -> Result<Self> {
        let Some(args) = args else {
            return Err(errno!(
                EINVAL,
                "the `lowerdir` mount option must be specified"
            ));
        };

        let mut options = Self {
            is_default_permissions: false,
            lower_dirs: Vec::new(),
            upper_dir: None,
            work_dir: None,
            is_forced_read_only: fs_flags.contains(FsFlags::RDONLY),
            xattr_namespace: XattrNamespace::Trusted,
            uuid_mode: None,
            xino_mode: None,
            redirect_dir: None,
            index: None,
            nfs_export: None,
            metacopy: None,
            verity: None,
            fsync_mode: None,
            is_lowerdir_plus_seen: false,
            is_nooverride_creds_seen: false,
        };

        for entry in args.split(',') {
            if entry.is_empty() {
                continue;
            }
            let (key, value) = entry
                .split_once('=')
                .map_or((entry, None), |(key, value)| (key, Some(value)));
            options.set_entry(key, value)?;
        }

        if options.lower_dirs.is_empty() {
            return Err(errno!(
                EINVAL,
                "the `lowerdir` mount option must be specified"
            ));
        }
        options.verify()?;
        Ok(options)
    }

    fn set_entry(&mut self, key: &str, value: Option<&str>) -> Result<()> {
        match key {
            "lowerdir" => {
                let value = Self::require_value(value)?;
                if !self.lower_dirs.is_empty() {
                    if self.is_lowerdir_plus_seen {
                        return Err(errno!(
                            EINVAL,
                            "the `lowerdir+` mount option cannot be combined with `lowerdir`"
                        ));
                    }
                    return Err(errno!(EINVAL, "duplicate overlay mount option"));
                }
                self.lower_dirs = value.split(':').map(str::to_string).collect();
                if self.lower_dirs.iter().any(|lower_dir| lower_dir.is_empty()) {
                    return Err(errno!(
                        EINVAL,
                        "the `lowerdir` value contains an empty layer path"
                    ));
                }
            }
            "lowerdir+" => {
                let value = Self::require_value(value)?;
                if !self.lower_dirs.is_empty() && !self.is_lowerdir_plus_seen {
                    return Err(errno!(
                        EINVAL,
                        "the `lowerdir+` mount option cannot be combined with `lowerdir`"
                    ));
                }
                self.is_lowerdir_plus_seen = true;
                self.lower_dirs.push(value.to_string());
            }
            "upperdir" => {
                Self::set_once(&mut self.upper_dir, Self::require_value(value)?.to_string())?
            }
            "workdir" => {
                Self::set_once(&mut self.work_dir, Self::require_value(value)?.to_string())?
            }
            "uuid" => Self::set_once(
                &mut self.uuid_mode,
                match Self::require_value(value)? {
                    "off" => UuidMode::Off,
                    "null" => UuidMode::Null,
                    "on" => UuidMode::On,
                    "auto" => UuidMode::Auto,
                    _ => return Err(errno!(EINVAL, "invalid overlay mount option value")),
                },
            )?,
            "xino" => Self::set_once(
                &mut self.xino_mode,
                match Self::require_value(value)? {
                    "off" => XinoMode::Off,
                    "auto" => XinoMode::Auto,
                    "on" => XinoMode::On,
                    _ => return Err(errno!(EINVAL, "invalid overlay mount option value")),
                },
            )?,
            "redirect_dir" => Self::set_once(
                &mut self.redirect_dir,
                match Self::require_value(value)? {
                    "off" => RedirectDirMode::Off,
                    "follow" => RedirectDirMode::Follow,
                    "nofollow" => RedirectDirMode::NoFollow,
                    "on" => RedirectDirMode::On,
                    _ => return Err(errno!(EINVAL, "invalid overlay mount option value")),
                },
            )?,
            "index" => Self::set_once(
                &mut self.index,
                Self::parse_bool(Self::require_value(value)?)?,
            )?,
            "nfs_export" => Self::set_once(
                &mut self.nfs_export,
                Self::parse_bool(Self::require_value(value)?)?,
            )?,
            "metacopy" => Self::set_once(
                &mut self.metacopy,
                Self::parse_bool(Self::require_value(value)?)?,
            )?,
            "verity" => Self::set_once(
                &mut self.verity,
                match Self::require_value(value)? {
                    "off" => VerityMode::Off,
                    "on" => VerityMode::On,
                    "require" => VerityMode::Require,
                    _ => return Err(errno!(EINVAL, "invalid overlay mount option value")),
                },
            )?,
            "fsync" => Self::set_once(
                &mut self.fsync_mode,
                match Self::require_value(value)? {
                    "volatile" => FsyncMode::Volatile,
                    "auto" => FsyncMode::Auto,
                    "strict" => FsyncMode::Strict,
                    _ => return Err(errno!(EINVAL, "invalid overlay mount option value")),
                },
            )?,
            "default_permissions" => {
                Self::require_bare(value)?;
                Self::set_flag_once(&mut self.is_default_permissions)?;
            }
            "userxattr" => {
                Self::require_bare(value)?;
                if self.xattr_namespace != XattrNamespace::Trusted {
                    return Err(errno!(EINVAL, "duplicate overlay mount option"));
                }
                self.xattr_namespace = XattrNamespace::User;
            }
            "volatile" => {
                Self::require_bare(value)?;
                Self::set_once(&mut self.fsync_mode, FsyncMode::Volatile)?;
            }
            "nooverride_creds" => {
                Self::require_bare(value)?;
                Self::set_flag_once(&mut self.is_nooverride_creds_seen)?;
            }
            _ => {
                return Err(errno!(EINVAL, "unknown overlay mount option"));
            }
        }
        Ok(())
    }

    fn require_value(value: Option<&str>) -> Result<&str> {
        let Some(value) = value else {
            return Err(errno!(EINVAL, "the overlay mount option requires a value"));
        };
        if value.is_empty() {
            return Err(errno!(
                EINVAL,
                "the overlay mount option requires a non-empty value"
            ));
        }
        Ok(value)
    }

    fn require_bare(value: Option<&str>) -> Result<()> {
        if value.is_some() {
            return Err(errno!(
                EINVAL,
                "the overlay mount option does not take a value"
            ));
        }
        Ok(())
    }

    fn parse_bool(value: &str) -> Result<bool> {
        match value {
            "on" => Ok(true),
            "off" => Ok(false),
            _ => Err(errno!(EINVAL, "invalid overlay mount option value")),
        }
    }

    fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<()> {
        if slot.is_some() {
            return Err(errno!(EINVAL, "duplicate overlay mount option"));
        }
        *slot = Some(value);
        Ok(())
    }

    fn set_flag_once(flag: &mut bool) -> Result<()> {
        if *flag {
            return Err(errno!(EINVAL, "duplicate overlay mount option"));
        }
        *flag = true;
        Ok(())
    }

    fn verify(&self) -> Result<()> {
        if self.upper_dir.is_some() != self.work_dir.is_some() {
            return Err(errno!(
                EINVAL,
                "the `workdir` mount option is required if and only if `upperdir` is specified"
            ));
        }
        if self.xattr_namespace == XattrNamespace::User
            && self
                .redirect_dir
                .is_some_and(|mode| mode != RedirectDirMode::NoFollow)
        {
            return Err(errno!(
                EINVAL,
                "conflicting overlay mount options: `userxattr` requires `redirect_dir=nofollow`"
            ));
        }
        if self.xattr_namespace == XattrNamespace::User && self.metacopy == Some(true) {
            return Err(errno!(
                EINVAL,
                "conflicting overlay mount options: `userxattr` and `metacopy=on`"
            ));
        }
        if self.metacopy == Some(true)
            && self
                .redirect_dir
                .is_some_and(|mode| mode != RedirectDirMode::On)
        {
            return Err(errno!(
                EINVAL,
                "conflicting overlay mount options: `metacopy=on` requires `redirect_dir=on`"
            ));
        }
        if self.nfs_export == Some(true) && self.index == Some(false) {
            return Err(errno!(
                EINVAL,
                "conflicting overlay mount options: `nfs_export=on` and `index=off`"
            ));
        }
        if self.nfs_export == Some(true) && self.metacopy == Some(true) {
            return Err(errno!(
                EINVAL,
                "conflicting overlay mount options: `nfs_export=on` and `metacopy=on`"
            ));
        }

        if let Some(mode) = self
            .redirect_dir
            .filter(|mode| *mode != RedirectDirMode::NoFollow)
        {
            warn!(
                "`redirect_dir={}`: degrading to redirect_dir=nofollow",
                mode.option_value()
            );
        }
        if self.index == Some(true) {
            warn!("`index=on`: degrading to index=off");
        }
        if self.nfs_export == Some(true) {
            warn!("`nfs_export=on`: degrading to nfs_export=off");
        }
        if self.metacopy == Some(true) {
            warn!("`metacopy=on`: degrading to metacopy=off");
        }
        if let Some(mode) = self.verity.filter(|mode| *mode != VerityMode::Off) {
            warn!("`verity={}`: degrading to verity=off", mode.option_value());
        }
        match self.fsync_mode {
            Some(FsyncMode::Strict) => {
                warn!("`fsync=strict`: metadata copy-up is not synced");
            }
            Some(FsyncMode::Volatile) => {
                warn!(
                    "`fsync=volatile`: not implemented; durability follows the underlying filesystem"
                );
            }
            Some(FsyncMode::Auto) | None => {}
        }
        if self.is_default_permissions {
            warn!("`default_permissions`: inert; the mount already behaves as requested");
        }
        Ok(())
    }
}

impl RedirectDirMode {
    fn option_value(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Follow => "follow",
            Self::NoFollow => "nofollow",
            Self::On => "on",
        }
    }
}

impl VerityMode {
    fn option_value(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Require => "require",
        }
    }
}

#[cfg(ktest)]
mod test {

    use ostd::prelude::ktest;

    use super::*;

    fn parse_expect_ok(args: &str, fs_flags: FsFlags) -> MountOptions {
        MountOptions::parse(Some(args), fs_flags).unwrap()
    }

    fn parse_expect_einval(args: Option<&str>, fs_flags: FsFlags) -> Error {
        let err = MountOptions::parse(args, fs_flags).unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);
        err
    }

    #[ktest]
    fn parse_requires_lowerdir() {
        parse_expect_einval(None, FsFlags::empty());
        parse_expect_einval(Some(""), FsFlags::empty());
        parse_expect_einval(Some(",,"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir="), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=a::b"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=:"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=a:"), FsFlags::empty());
    }

    #[ktest]
    fn parse_lowerdir_layer_list() {
        let options = parse_expect_ok("lowerdir=a", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["a"]);
        let options = parse_expect_ok("lowerdir=a:b:c", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["a", "b", "c"]);
        let read_only = parse_expect_ok("lowerdir=l", FsFlags::RDONLY);
        assert!(read_only.is_forced_read_only);
        let writable = parse_expect_ok("lowerdir=l", FsFlags::empty());
        assert!(!writable.is_forced_read_only);
    }

    #[ktest]
    fn parse_upperdir_workdir_pairing() {
        parse_expect_einval(Some("lowerdir=l,upperdir=u"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,workdir=w"), FsFlags::empty());
        let options = parse_expect_ok("lowerdir=l,upperdir=u,workdir=w", FsFlags::empty());
        assert_eq!(options.upper_dir.as_deref(), Some("u"));
        assert_eq!(options.work_dir.as_deref(), Some("w"));
    }

    #[ktest]
    fn parse_rejects_duplicate_keys() {
        parse_expect_einval(Some("lowerdir=a,lowerdir=b"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,upperdir=u,upperdir=v"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,workdir=w,workdir=v"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,uuid=on,uuid=off"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,xino=on,xino=off"), FsFlags::empty());
        parse_expect_einval(
            Some("lowerdir=l,default_permissions,default_permissions"),
            FsFlags::empty(),
        );
        parse_expect_einval(Some("lowerdir=l,userxattr,userxattr"), FsFlags::empty());
    }

    #[ktest]
    fn parse_rejects_bare_valued_keys() {
        parse_expect_einval(Some("lowerdir"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,upperdir"), FsFlags::empty());
        parse_expect_einval(Some(",workdir"), FsFlags::empty());
        parse_expect_einval(Some(",uuid"), FsFlags::empty());
        parse_expect_einval(Some(",xino"), FsFlags::empty());
        parse_expect_einval(Some("foo=bar"), FsFlags::empty());
        parse_expect_einval(Some("foo"), FsFlags::empty());
        parse_expect_einval(Some("=value"), FsFlags::empty());
        parse_expect_einval(Some("LOWERDIR=l"), FsFlags::empty());
    }

    #[ktest]
    fn parse_valueless_keys() {
        let options = parse_expect_ok("lowerdir=l,default_permissions", FsFlags::empty());
        assert!(options.is_default_permissions);
        let options = parse_expect_ok("lowerdir=l,userxattr", FsFlags::empty());
        assert_eq!(options.xattr_namespace, XattrNamespace::User);
        let options = parse_expect_ok("lowerdir=l,default_permissions,userxattr", FsFlags::empty());
        assert!(options.is_default_permissions);
        assert_eq!(options.xattr_namespace, XattrNamespace::User);
        parse_expect_einval(Some("default_permissions=x"), FsFlags::empty());
        parse_expect_einval(Some("userxattr=1"), FsFlags::empty());
        parse_expect_einval(Some("userxattr="), FsFlags::empty());
    }

    #[ktest]
    fn parse_uuid_and_xino_values() {
        assert_eq!(
            parse_expect_ok("lowerdir=l,uuid=off", FsFlags::empty()).uuid_mode,
            Some(UuidMode::Off)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,uuid=null", FsFlags::empty()).uuid_mode,
            Some(UuidMode::Null)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,uuid=on", FsFlags::empty()).uuid_mode,
            Some(UuidMode::On)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,uuid=auto", FsFlags::empty()).uuid_mode,
            Some(UuidMode::Auto)
        );
        parse_expect_einval(Some("uuid="), FsFlags::empty());
        parse_expect_einval(Some("uuid=ON"), FsFlags::empty());
        parse_expect_einval(Some("uuid=yes"), FsFlags::empty());
        assert_eq!(
            parse_expect_ok("lowerdir=l,xino=off", FsFlags::empty()).xino_mode,
            Some(XinoMode::Off)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,xino=auto", FsFlags::empty()).xino_mode,
            Some(XinoMode::Auto)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,xino=on", FsFlags::empty()).xino_mode,
            Some(XinoMode::On)
        );
        parse_expect_einval(Some("xino="), FsFlags::empty());
        parse_expect_einval(Some("xino=ON"), FsFlags::empty());
        parse_expect_einval(Some("xino=1"), FsFlags::empty());
        parse_expect_einval(Some("xino=on "), FsFlags::empty());
    }

    #[ktest]
    fn parse_literal_value_semantics() {
        let options = parse_expect_ok("lowerdir=a=b", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["a=b"]);
        let options = parse_expect_ok("lowerdir=\"a\"", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["\"a\""]);
        let options = parse_expect_ok("lowerdir=a b", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["a b"]);
        let options = parse_expect_ok("lowerdir= a", FsFlags::empty());
        assert_eq!(options.lower_dirs, [" a"]);
        parse_expect_einval(Some("lowerdir=l,uuid=on "), FsFlags::empty());
        parse_expect_einval(Some("lowerdir=a,b"), FsFlags::empty());
        let options = parse_expect_ok(
            ",,lowerdir=l,,userxattr,,default_permissions,,",
            FsFlags::empty(),
        );
        assert_eq!(options.xattr_namespace, XattrNamespace::User);
        assert!(options.is_default_permissions);
    }

    #[ktest]
    fn parse_redirect_dir_domain() {
        assert_eq!(
            parse_expect_ok("lowerdir=l,redirect_dir=on", FsFlags::empty()).redirect_dir,
            Some(RedirectDirMode::On)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,redirect_dir=follow", FsFlags::empty()).redirect_dir,
            Some(RedirectDirMode::Follow)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,redirect_dir=nofollow", FsFlags::empty()).redirect_dir,
            Some(RedirectDirMode::NoFollow)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,redirect_dir=off", FsFlags::empty()).redirect_dir,
            Some(RedirectDirMode::Off)
        );
        parse_expect_einval(Some("redirect_dir="), FsFlags::empty());
        parse_expect_einval(Some("redirect_dir=ON"), FsFlags::empty());
        parse_expect_einval(Some("redirect_dir=yes"), FsFlags::empty());
        parse_expect_einval(Some("redirect_dir"), FsFlags::empty());
        parse_expect_einval(Some("redirect_dir=on,redirect_dir=off"), FsFlags::empty());
    }

    #[ktest]
    fn parse_bool_option_domains() {
        assert_eq!(
            parse_expect_ok("lowerdir=l,index=on", FsFlags::empty()).index,
            Some(true)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,index=off", FsFlags::empty()).index,
            Some(false)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,nfs_export=on", FsFlags::empty()).nfs_export,
            Some(true)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,nfs_export=off", FsFlags::empty()).nfs_export,
            Some(false)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,metacopy=on", FsFlags::empty()).metacopy,
            Some(true)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,metacopy=off", FsFlags::empty()).metacopy,
            Some(false)
        );
        parse_expect_einval(Some("index=1"), FsFlags::empty());
        parse_expect_einval(Some("index=ON"), FsFlags::empty());
        parse_expect_einval(Some("index="), FsFlags::empty());
        parse_expect_einval(Some("index"), FsFlags::empty());
        parse_expect_einval(Some("index=on,index=off"), FsFlags::empty());
    }

    #[ktest]
    fn parse_verity_and_fsync_domains() {
        assert_eq!(
            parse_expect_ok("lowerdir=l,verity=off", FsFlags::empty()).verity,
            Some(VerityMode::Off)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,verity=on", FsFlags::empty()).verity,
            Some(VerityMode::On)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,verity=require", FsFlags::empty()).verity,
            Some(VerityMode::Require)
        );
        parse_expect_einval(Some("verity=required"), FsFlags::empty());
        parse_expect_einval(Some("verity="), FsFlags::empty());
        parse_expect_einval(Some("verity"), FsFlags::empty());
        parse_expect_einval(Some("verity=on,verity=off"), FsFlags::empty());
        assert_eq!(
            parse_expect_ok("lowerdir=l,fsync=auto", FsFlags::empty()).fsync_mode,
            Some(FsyncMode::Auto)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,fsync=strict", FsFlags::empty()).fsync_mode,
            Some(FsyncMode::Strict)
        );
        assert_eq!(
            parse_expect_ok("lowerdir=l,fsync=volatile", FsFlags::empty()).fsync_mode,
            Some(FsyncMode::Volatile)
        );
        parse_expect_einval(Some("fsync=0"), FsFlags::empty());
        parse_expect_einval(Some("fsync="), FsFlags::empty());
        parse_expect_einval(Some("fsync"), FsFlags::empty());
        parse_expect_einval(Some("fsync=auto,fsync=strict"), FsFlags::empty());
    }

    #[ktest]
    fn parse_volatile_alias() {
        assert_eq!(
            parse_expect_ok("lowerdir=l,volatile", FsFlags::empty()).fsync_mode,
            Some(FsyncMode::Volatile)
        );
        parse_expect_einval(Some("volatile=1"), FsFlags::empty());
        parse_expect_einval(Some("volatile="), FsFlags::empty());
        parse_expect_einval(Some("volatile,volatile"), FsFlags::empty());
        parse_expect_einval(Some("volatile,fsync=auto"), FsFlags::empty());
        parse_expect_einval(Some("fsync=auto,volatile"), FsFlags::empty());
    }

    #[ktest]
    fn parse_override_creds_forms() {
        parse_expect_einval(Some("lowerdir=l,override_creds"), FsFlags::empty());
        parse_expect_einval(Some("override_creds=on"), FsFlags::empty());
        let options = parse_expect_ok("lowerdir=l,nooverride_creds", FsFlags::empty());
        assert_eq!(options.redirect_dir, None);
        assert_eq!(options.index, None);
        assert_eq!(options.nfs_export, None);
        assert_eq!(options.metacopy, None);
        assert_eq!(options.verity, None);
        assert_eq!(options.fsync_mode, None);
        parse_expect_einval(Some("nooverride_creds=x"), FsFlags::empty());
        parse_expect_einval(Some("nonooverride_creds"), FsFlags::empty());
        parse_expect_einval(Some("nooverride_creds,nooverride_creds"), FsFlags::empty());
    }

    #[ktest]
    fn parse_lowerdir_plus_append() {
        let options = parse_expect_ok("lowerdir+=/a,lowerdir+=/b", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["/a", "/b"]);
        let options = parse_expect_ok("lowerdir+=/a:b", FsFlags::empty());
        assert_eq!(options.lower_dirs, ["/a:b"]);
        parse_expect_einval(Some("lowerdir=/l,lowerdir+=/a"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir+=/a,lowerdir=/l"), FsFlags::empty());
        parse_expect_einval(Some("lowerdir+="), FsFlags::empty());
        parse_expect_einval(Some("lowerdir+=/a,b"), FsFlags::empty());
    }

    #[ktest]
    fn parse_datadir_plus_rejected() {
        parse_expect_einval(Some("lowerdir=l,datadir+=/d"), FsFlags::empty());
    }

    #[ktest]
    fn parse_new_key_conflicts() {
        parse_expect_einval(
            Some("lowerdir=l,userxattr,redirect_dir=on"),
            FsFlags::empty(),
        );
        parse_expect_einval(
            Some("lowerdir=l,userxattr,redirect_dir=follow"),
            FsFlags::empty(),
        );
        parse_expect_einval(
            Some("lowerdir=l,userxattr,redirect_dir=off"),
            FsFlags::empty(),
        );
        parse_expect_ok(
            "lowerdir=l,userxattr,redirect_dir=nofollow",
            FsFlags::empty(),
        );
        parse_expect_einval(Some("lowerdir=l,userxattr,metacopy=on"), FsFlags::empty());
        parse_expect_ok("lowerdir=l,userxattr,metacopy=off", FsFlags::empty());
        parse_expect_einval(
            Some("lowerdir=l,metacopy=on,redirect_dir=nofollow"),
            FsFlags::empty(),
        );
        parse_expect_einval(
            Some("lowerdir=l,metacopy=on,redirect_dir=off"),
            FsFlags::empty(),
        );
        parse_expect_einval(
            Some("lowerdir=l,metacopy=on,redirect_dir=follow"),
            FsFlags::empty(),
        );
        parse_expect_ok("lowerdir=l,metacopy=on,redirect_dir=on", FsFlags::empty());
        parse_expect_einval(Some("lowerdir=l,nfs_export=on,index=off"), FsFlags::empty());
        parse_expect_einval(
            Some("lowerdir=l,nfs_export=on,metacopy=on"),
            FsFlags::empty(),
        );
    }
}
