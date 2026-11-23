#![short_vis_path::add(overlayfs)]

use core::sync::atomic::{AtomicU64, Ordering};

use device_id::DeviceId;
use hashbrown::HashMap;

use super::{copyup::workdir::WorkdirTemp, xattr::OverlayXattrType};
use crate::{
    fs::{
        fs_impls::overlayfs::{
            fs::{
                OverlayFs,
                mount::{XinoMode, policy::MountPolicy},
            },
            layer::{Layer, LayerStack},
            real::{RealObject, RealObjectStack},
        },
        pseudofs::AnonDeviceId,
        vfs::{file_system::FileSystem, path::Dentry},
    },
    prelude::*,
};

#[derive(Debug)]
pub(in overlayfs) struct IdentityPolicy {
    mode: IdentityMode,
    overlay_device: AnonDeviceId,
    fallback_ino: Option<AtomicU64>,
    upper_numbers: Option<LayerNumbers>,
    lower_numbers: Vec<LayerNumbers>,
}

#[derive(Debug)]
enum IdentityMode {
    SameFs,
    Fallback,
    Xino { xino_shift: u32 },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct ObjectRealId {
    fsid: u64,
    real_ino: u64,
}

impl ObjectRealId {
    fn encode_xino(&self, xino_shift: u32) -> Option<u64> {
        if self.real_ino >> xino_shift != 0 {
            return None;
        }
        let fsid_shift = xino_shift + 1;
        Some((self.fsid << fsid_shift) | self.real_ino)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ObjectVisibleId {
    pub(super) dev: DeviceId,
    pub(super) ino: u64,
}

#[derive(Clone, Copy, Debug)]
struct LayerNumbers {
    fsid: u64,
    container_dev_id: DeviceId,
    root_ino: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct ObjectOriginRecord {
    magic: u64,
    container_dev_id: u64,
    root_ino: u64,
    real_ino: u64,
}

impl ObjectOriginRecord {
    const MAGIC: u64 = 0x0000_00fb;

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != size_of::<Self>() {
            return None;
        }
        let record = Self::from_bytes(bytes);
        if record.magic != Self::MAGIC
            || DeviceId::from_encoded_u64(record.container_dev_id).is_none()
        {
            return None;
        }
        Some(record)
    }
}

impl IdentityPolicy {
    fn assemble(
        overlay_device: AnonDeviceId,
        xino_mode: XinoMode,
        fs_count: u64,
        is_effective_read_only: bool,
        can_store_private_xattr: bool,
        upper_numbers: Option<LayerNumbers>,
        lower_numbers: Vec<LayerNumbers>,
    ) -> Self {
        let is_all_layers_same_fs = fs_count == 1;
        let mode = if is_all_layers_same_fs && xino_mode != XinoMode::On {
            IdentityMode::SameFs
        } else if xino_mode == XinoMode::Off
            || (xino_mode == XinoMode::Auto && !(is_effective_read_only || can_store_private_xattr))
        {
            IdentityMode::Fallback
        } else {
            IdentityMode::Xino {
                xino_shift: 64 - (fs_count.max(2) - 1).ilog2() - 2,
            }
        };
        let fallback_ino = match mode {
            IdentityMode::SameFs => None,
            IdentityMode::Fallback | IdentityMode::Xino { .. } => Some(AtomicU64::new(1)),
        };
        Self {
            mode,
            overlay_device,
            fallback_ino,
            upper_numbers,
            lower_numbers,
        }
    }

    pub(super) fn project(&self, real_id: ObjectRealId, is_directory: bool) -> ObjectVisibleId {
        let layer_pair = ObjectVisibleId {
            dev: self.device_of_fsid(real_id.fsid),
            ino: real_id.real_ino,
        };
        match self.mode {
            IdentityMode::SameFs => layer_pair,
            IdentityMode::Fallback => {
                if is_directory {
                    ObjectVisibleId {
                        dev: self.overlay_device.id(),
                        ino: self.allocate_fallback_ino(),
                    }
                } else {
                    layer_pair
                }
            }
            IdentityMode::Xino { xino_shift } => match real_id.encode_xino(xino_shift) {
                Some(ino) => ObjectVisibleId {
                    dev: self.overlay_device.id(),
                    ino,
                },
                None if is_directory => ObjectVisibleId {
                    dev: self.overlay_device.id(),
                    ino: self.xino_fallback_ino(xino_shift),
                },
                None => layer_pair,
            },
        }
    }

    pub(super) fn real_id_of(&self, real: &RealObject) -> ObjectRealId {
        ObjectRealId {
            fsid: self.layer(real.layer_index()).fsid,
            real_ino: real.real_inode().ino(),
        }
    }

    fn origin_record_of(&self, source: &RealObject) -> ObjectOriginRecord {
        let layer = self.layer(source.layer_index());
        ObjectOriginRecord {
            magic: ObjectOriginRecord::MAGIC,
            container_dev_id: layer.container_dev_id.as_encoded_u64(),
            root_ino: layer.root_ino,
            real_ino: source.real_inode().ino(),
        }
    }

    pub(super) fn is_same_fs_passthrough(&self) -> bool {
        matches!(self.mode, IdentityMode::SameFs)
    }

    fn layer(&self, layer_index: usize) -> &LayerNumbers {
        match layer_index {
            0 => self
                .upper_numbers
                .as_ref()
                .expect("a real object with layer index 0 references the upper layer"),
            _ => self
                .lower_numbers
                .get(layer_index - 1)
                .expect("a real object references a configured lower layer"),
        }
    }

    fn device_of_fsid(&self, fsid: u64) -> DeviceId {
        self.upper_numbers
            .iter()
            .chain(self.lower_numbers.iter())
            .find(|layer| layer.fsid == fsid)
            .expect("a published fsid belongs to a configured layer")
            .container_dev_id
    }

    fn fsid_of_origin(&self, origin: &ObjectOriginRecord) -> Option<u64> {
        let mut matched: Option<u64> = None;
        for layer in self.lower_numbers.iter() {
            if layer.container_dev_id.as_encoded_u64() != origin.container_dev_id
                || layer.root_ino != origin.root_ino
            {
                continue;
            }
            match matched {
                None => matched = Some(layer.fsid),
                Some(existing) if existing == layer.fsid => {}
                Some(_) => return None,
            }
        }
        matched
    }

    fn allocate_fallback_ino(&self) -> u64 {
        let range = self
            .fallback_ino
            .as_ref()
            .expect("a form that allocates numbers keeps one");
        range.fetch_add(1, Ordering::Relaxed)
    }

    fn xino_fallback_ino(&self, xino_shift: u32) -> u64 {
        let cleared_bit = 1 << xino_shift;
        let ino = self.allocate_fallback_ino();
        cleared_bit | (ino & (cleared_bit - 1))
    }
}

impl OverlayFs {
    pub(in overlayfs) fn new_identity_policy(
        overlay_device: AnonDeviceId,
        layers: &LayerStack,
        xino_mode: XinoMode,
        is_effective_read_only: bool,
        policy: &MountPolicy,
    ) -> IdentityPolicy {
        let mut fses: Vec<Arc<dyn FileSystem>> = Vec::new();
        let mut fsid_by_address: HashMap<usize, u64> = HashMap::new();
        let mut assign_fsid = |layer: &Layer, fses: &mut Vec<Arc<dyn FileSystem>>| -> u64 {
            let address = Arc::as_ptr(layer.fs()) as *const () as usize;
            if let Some(fsid) = fsid_by_address.get(&address) {
                return *fsid;
            }
            let fsid = fses.len() as u64;
            fses.push(layer.fs().clone());
            fsid_by_address.insert(address, fsid);
            fsid
        };
        let upper_numbers = layers.upper_layer().ok().map(|upper| LayerNumbers {
            fsid: assign_fsid(upper, &mut fses),
            container_dev_id: upper.container_dev_id(),
            root_ino: upper.root_dentry().inode().ino(),
        });
        let lower_numbers: Vec<LayerNumbers> = layers
            .lower_layers()
            .iter()
            .map(|lower| LayerNumbers {
                fsid: assign_fsid(lower, &mut fses),
                container_dev_id: lower.container_dev_id(),
                root_ino: lower.root_dentry().inode().ino(),
            })
            .collect();
        IdentityPolicy::assemble(
            overlay_device,
            xino_mode,
            fses.len() as u64,
            is_effective_read_only,
            policy.can_store_private_xattr(),
            upper_numbers,
            lower_numbers,
        )
    }

    pub(super) fn origin_of(&self, upper: &Dentry) -> Option<ObjectRealId> {
        let mut value = [0u8; size_of::<ObjectOriginRecord>()];
        let read = OverlayXattrType::Origin.get_value_from(
            upper,
            self.policy().xattr_namespace(),
            &mut value,
        );
        let record = match read {
            Ok(written) => ObjectOriginRecord::decode(&value[..written])?,
            Err(err)
                if matches!(
                    err.error(),
                    Errno::ENODATA | Errno::EOPNOTSUPP | Errno::ERANGE
                ) =>
            {
                return None;
            }
            Err(err) => {
                warn!(
                    "failed to read the origin record; treating the object as a pure upper: {:?}",
                    err
                );
                return None;
            }
        };
        let fsid = self.identity().fsid_of_origin(&record)?;
        Some(ObjectRealId {
            fsid,
            real_ino: record.real_ino,
        })
    }

    pub(super) fn real_id_of_stack(&self, facts: &RealObjectStack) -> ObjectRealId {
        let visible = facts.visible_source();
        let is_directory = visible.real_inode().type_().is_directory();
        let identity = self.identity();
        if facts.upper.is_none() {
            identity.real_id_of(visible)
        } else if is_directory {
            match facts.lowers.first() {
                Some(topmost_lower) => identity.real_id_of(topmost_lower),
                None => identity.real_id_of(visible),
            }
        } else {
            self.origin_of(visible.dentry())
                .unwrap_or_else(|| identity.real_id_of(visible))
        }
    }

    pub(super) fn record_origin(&self, temp: &WorkdirTemp, source: &RealObject) -> Result<bool> {
        if !self.policy().can_store_private_xattr() {
            return Ok(false);
        }
        let record = self.identity().origin_record_of(source);
        match OverlayXattrType::Origin.set_value_on(
            temp.dentry(),
            self.policy().xattr_namespace(),
            Some(record.as_bytes()),
        ) {
            Ok(()) => Ok(true),
            Err(err) if matches!(err.error(), Errno::EOPNOTSUPP | Errno::EPERM) => Ok(false),
            Err(err) => Err(err),
        }
    }
}

#[cfg(ktest)]
mod test {

    use ostd::prelude::ktest;

    use super::*;

    fn dev(major: u16, minor: u32) -> DeviceId {
        DeviceId::new(
            device_id::MajorId::new(major),
            device_id::MinorId::new(minor),
        )
    }

    fn build_policy(
        xino_mode: XinoMode,
        fs_count: u64,
        is_effective_read_only: bool,
        can_store_private_xattr: bool,
        upper_numbers: Option<LayerNumbers>,
        lower_numbers: Vec<LayerNumbers>,
    ) -> IdentityPolicy {
        IdentityPolicy::assemble(
            AnonDeviceId::acquire()
                .expect("an anonymous device ID should be available for the unit test"),
            xino_mode,
            fs_count,
            is_effective_read_only,
            can_store_private_xattr,
            upper_numbers,
            lower_numbers,
        )
    }

    fn numbers(fsid: u64, container_dev_id: DeviceId, root_ino: u64) -> LayerNumbers {
        LayerNumbers {
            fsid,
            container_dev_id,
            root_ino,
        }
    }

    fn real_id(fsid: u64, real_ino: u64) -> ObjectRealId {
        ObjectRealId { fsid, real_ino }
    }

    fn record(
        container_dev_id: DeviceId,
        lower_layer_root_ino: u64,
        real_ino: u64,
    ) -> ObjectOriginRecord {
        let layer = numbers(0, container_dev_id, lower_layer_root_ino);
        ObjectOriginRecord {
            magic: ObjectOriginRecord::MAGIC,
            container_dev_id: layer.container_dev_id.as_encoded_u64(),
            root_ino: layer.root_ino,
            real_ino,
        }
    }

    fn valid_wire() -> Vec<u8> {
        record(dev(1, 1), 100, 0x1234).as_bytes().to_vec()
    }

    #[ktest]
    fn identity_mode_resolves_from_mount_facts() {
        let same_fs_off = build_policy(XinoMode::Off, 1, false, false, None, Vec::new());
        assert!(matches!(same_fs_off.mode, IdentityMode::SameFs));
        assert!(same_fs_off.fallback_ino.is_none());

        let same_fs_auto = build_policy(XinoMode::Auto, 1, false, false, None, Vec::new());
        assert!(matches!(same_fs_auto.mode, IdentityMode::SameFs));
        assert!(same_fs_auto.fallback_ino.is_none());

        let spanning_off = build_policy(XinoMode::Off, 2, false, false, None, Vec::new());
        assert!(matches!(spanning_off.mode, IdentityMode::Fallback));
        assert!(spanning_off.fallback_ino.is_some());

        let spanning_auto = build_policy(XinoMode::Auto, 2, false, false, None, Vec::new());
        assert!(matches!(spanning_auto.mode, IdentityMode::Fallback));
        assert!(spanning_auto.fallback_ino.is_some());

        let auto_read_only = build_policy(XinoMode::Auto, 2, true, false, None, Vec::new());
        assert!(matches!(auto_read_only.mode, IdentityMode::Xino { .. }));
        assert!(auto_read_only.fallback_ino.is_some());

        let auto_recording = build_policy(XinoMode::Auto, 2, false, true, None, Vec::new());
        assert!(matches!(auto_recording.mode, IdentityMode::Xino { .. }));
        assert!(auto_recording.fallback_ino.is_some());

        let on_same_fs = build_policy(XinoMode::On, 1, false, false, None, Vec::new());
        assert!(matches!(on_same_fs.mode, IdentityMode::Xino { .. }));
        assert!(on_same_fs.fallback_ino.is_some());

        let on_spanning = build_policy(XinoMode::On, 2, true, true, None, Vec::new());
        assert!(matches!(on_spanning.mode, IdentityMode::Xino { .. }));
        assert!(on_spanning.fallback_ino.is_some());
    }

    #[ktest]
    fn xino_on_forces_encoding_on_same_fs() {
        let upper = numbers(0, dev(1, 1), 100);
        let policy = build_policy(XinoMode::On, 1, false, false, Some(upper), Vec::new());
        let encoded = ObjectVisibleId {
            dev: policy.overlay_device.id(),
            ino: 777,
        };
        assert_eq!(policy.project(real_id(0, 777), false), encoded);
        assert_eq!(policy.project(real_id(0, 777), true), encoded);
        assert!(matches!(policy.mode, IdentityMode::Xino { .. }));
    }

    #[ktest]
    fn xino_encodes_fsid_in_high_bits() {
        for (fs_count, expected_shift) in [(2u64, 62u32), (3, 61), (5, 60), (9, 59)] {
            let policy = build_policy(
                XinoMode::On,
                fs_count,
                false,
                false,
                None,
                vec![numbers(3, dev(1, 1), 200)],
            );
            let resolved = match &policy.mode {
                IdentityMode::Xino { xino_shift } => *xino_shift,
                _ => unreachable!("`xino=on` over more than one filesystem keeps payload bits"),
            };
            assert_eq!(resolved, expected_shift);
        }

        let policy = build_policy(
            XinoMode::On,
            4,
            false,
            false,
            None,
            vec![numbers(3, dev(1, 1), 200)],
        );
        let xino_shift = match &policy.mode {
            IdentityMode::Xino { xino_shift } => *xino_shift,
            _ => unreachable!("`xino=on` over more than one filesystem keeps payload bits"),
        };
        let encoded = policy.project(real_id(3, 0x1234), false);
        assert_eq!(
            encoded,
            ObjectVisibleId {
                dev: policy.overlay_device.id(),
                ino: (3 << (xino_shift + 1)) | 0x1234
            }
        );
        assert_eq!(encoded.ino >> (xino_shift + 1), 3);
        assert_eq!(encoded.ino & ((1 << xino_shift) - 1), 0x1234);
        assert_eq!(encoded.ino & (1 << xino_shift), 0);

        let auto_policy = build_policy(
            XinoMode::Auto,
            4,
            false,
            true,
            None,
            vec![numbers(3, dev(1, 1), 200)],
        );
        let auto_encoded = auto_policy.project(real_id(3, 0x1234), false);
        assert_eq!(auto_encoded.ino, encoded.ino);
        assert_eq!(auto_encoded.dev, auto_policy.overlay_device.id());
        assert!(matches!(auto_policy.mode, IdentityMode::Xino { .. }));
    }

    #[ktest]
    fn xino_off_or_overflow_takes_fallback() {
        let policy = build_policy(
            XinoMode::On,
            4,
            false,
            false,
            None,
            vec![numbers(3, dev(1, 1), 200)],
        );
        let xino_shift = match &policy.mode {
            IdentityMode::Xino { xino_shift } => *xino_shift,
            _ => unreachable!("`xino=on` over more than one filesystem keeps payload bits"),
        };
        let oversized_ino = 1u64 << xino_shift;
        assert_eq!(
            policy.project(real_id(3, oversized_ino), false),
            ObjectVisibleId {
                dev: dev(1, 1),
                ino: oversized_ino
            }
        );
        let fallback = policy.project(real_id(3, oversized_ino), true);
        assert_eq!(
            fallback,
            ObjectVisibleId {
                dev: policy.overlay_device.id(),
                ino: (1 << xino_shift) | 1
            }
        );
        assert_ne!(fallback.ino & (1 << xino_shift), 0);
        assert_ne!(
            fallback,
            policy.project(real_id(3, oversized_ino - 1), false)
        );

        let off_policy = build_policy(
            XinoMode::Off,
            4,
            false,
            false,
            None,
            vec![numbers(3, dev(1, 1), 200)],
        );
        assert_eq!(
            off_policy.project(real_id(3, 7), false),
            ObjectVisibleId {
                dev: dev(1, 1),
                ino: 7
            }
        );
        assert_eq!(
            off_policy.project(real_id(3, 7), true),
            ObjectVisibleId {
                dev: off_policy.overlay_device.id(),
                ino: 1
            }
        );
    }

    #[ktest]
    fn fallback_ino_allocates_from_one() {
        let policy = build_policy(
            XinoMode::Off,
            4,
            false,
            false,
            None,
            vec![numbers(3, dev(1, 1), 200)],
        );
        let first = policy.project(real_id(3, 7), true);
        let second = policy.project(real_id(3, 7), true);
        let third = policy.project(real_id(3, 7), true);
        assert_eq!(
            first,
            ObjectVisibleId {
                dev: policy.overlay_device.id(),
                ino: 1,
            }
        );
        assert_eq!(
            second,
            ObjectVisibleId {
                dev: policy.overlay_device.id(),
                ino: 2,
            }
        );
        assert_eq!(
            third,
            ObjectVisibleId {
                dev: policy.overlay_device.id(),
                ino: 3,
            }
        );
    }

    #[ktest]
    fn origin_record_roundtrip_preserves_identity() {
        let row = numbers(3, dev(1, 1), 100);
        let record = ObjectOriginRecord {
            magic: ObjectOriginRecord::MAGIC,
            container_dev_id: row.container_dev_id.as_encoded_u64(),
            root_ino: row.root_ino,
            real_ino: 0x1234,
        };
        let wire = record.as_bytes().to_vec();
        assert_eq!(wire.len(), 32);
        let decoded = ObjectOriginRecord::decode(&wire).expect("a fresh record decodes");
        assert_eq!(decoded.container_dev_id, dev(1, 1).as_encoded_u64());
        assert_eq!(decoded.root_ino, 100);
        assert_eq!(decoded.real_ino, 0x1234);

        let policy = build_policy(XinoMode::On, 4, false, false, None, vec![row]);
        assert_eq!(policy.fsid_of_origin(&decoded), Some(3));
    }

    #[ktest]
    fn origin_record_decode_rejects_malformed() {
        let mut short = valid_wire();
        short.truncate(31);
        assert!(ObjectOriginRecord::decode(&short).is_none());
        let mut long = valid_wire();
        long.push(0);
        assert!(ObjectOriginRecord::decode(&long).is_none());
        let mut word = valid_wire();
        word[0] ^= 0xff;
        assert!(ObjectOriginRecord::decode(&word).is_none());
        let mut versioned = valid_wire();
        versioned[4] = 3;
        assert!(ObjectOriginRecord::decode(&versioned).is_none());
        let mut invalid_dev = valid_wire();
        invalid_dev[8..16]
            .copy_from_slice(&device_id::encode_device_numbers(0x1000, 0).to_ne_bytes());
        assert!(ObjectOriginRecord::decode(&invalid_dev).is_none());
    }

    #[ktest]
    fn layer_table_resolves_persistent_names() {
        let policy = build_policy(
            XinoMode::On,
            3,
            false,
            false,
            Some(numbers(0, dev(9, 9), 900)),
            vec![numbers(1, dev(1, 1), 100), numbers(2, dev(2, 2), 200)],
        );
        assert_eq!(policy.fsid_of_origin(&record(dev(1, 1), 100, 7)), Some(1));
        assert_eq!(policy.fsid_of_origin(&record(dev(2, 2), 200, 7)), Some(2));
        assert_eq!(policy.fsid_of_origin(&record(dev(1, 1), 300, 7)), None);
        assert_eq!(policy.fsid_of_origin(&record(dev(9, 9), 900, 7)), None);

        let ambiguous = build_policy(
            XinoMode::On,
            3,
            false,
            false,
            None,
            vec![numbers(1, dev(1, 1), 100), numbers(2, dev(1, 1), 100)],
        );
        assert_eq!(ambiguous.fsid_of_origin(&record(dev(1, 1), 100, 7)), None);

        let lower_only = build_policy(
            XinoMode::On,
            3,
            false,
            false,
            None,
            vec![numbers(1, dev(1, 1), 100), numbers(2, dev(2, 2), 200)],
        );
        assert_eq!(lower_only.layer(1).fsid, 1);
        assert_eq!(lower_only.layer(1).root_ino, 100);
        assert_eq!(lower_only.layer(2).fsid, 2);
        assert_eq!(lower_only.layer(2).root_ino, 200);
    }

    #[ktest]
    fn origin_record_resolves_to_layer() {
        let row = numbers(3, dev(1, 1), 100);
        let policy = build_policy(
            XinoMode::On,
            5,
            false,
            false,
            None,
            vec![row, numbers(4, dev(2, 2), 200)],
        );
        let origin = ObjectOriginRecord {
            magic: ObjectOriginRecord::MAGIC,
            container_dev_id: row.container_dev_id.as_encoded_u64(),
            root_ino: row.root_ino,
            real_ino: 0x1234,
        };
        let resolved = ObjectRealId {
            fsid: policy
                .fsid_of_origin(&origin)
                .expect("the record names one lower"),
            real_ino: origin.real_ino,
        };
        assert_eq!(resolved, real_id(3, 0x1234));
    }
}
