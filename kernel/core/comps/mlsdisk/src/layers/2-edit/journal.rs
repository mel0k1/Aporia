use core::marker::PhantomData;

use lending_iterator::LendingIterator;
use ostd_pod::{IntoBytes, Pod};
use serde::{
    Deserialize, Serialize,
    de::{VariantAccess, Visitor},
};

use super::{Edit, EditGroup};
use crate::{
    layers::{
        bio::{BlockRing, BlockSet, Buf},
        crypto::{CryptoBlob, CryptoChain, Key, Mac},
    },
    prelude::*,
};

pub(crate) struct EditJournal<E: Edit<S>, S, D, P> {
    state: S,
    journal_chain: CryptoChain<BlockRing<D>>,
    snapshots: SnapshotManager<S, D>,
    compaction_policy: P,
    curr_edit_group: Option<EditGroup<E, S>>,
    write_buf: WriteBuf<E, S>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct EditJournalMeta {
    pub snapshot_area_nblocks: usize,
    pub snapshot_area_keys: [Key; 2],
    pub journal_area_nblocks: usize,
    pub journal_area_key: Key,
}

impl EditJournalMeta {
    pub(crate) fn total_nblocks(&self) -> usize {
        self.snapshot_area_nblocks * 2 + self.journal_area_nblocks
    }
}

impl<E, S, D, P> EditJournal<E, S, D, P>
where
    E: Edit<S>,
    S: Serialize + for<'de> Deserialize<'de> + Clone,
    D: BlockSet,
    P: CompactPolicy<E, S>,
{
    pub(crate) fn format(
        disk: D,
        init_state: S,
        state_max_nbytes: usize,
        mut compaction_policy: P,
    ) -> Result<EditJournal<E, S, D, P>> {
        let snapshots = SnapshotManager::create(&disk, &init_state, state_max_nbytes)?;

        let mut journal_chain = {
            let chain_set = disk.subset(snapshots.nblocks() * 2..disk.nblocks())?;
            let block_ring = BlockRing::new(chain_set);
            block_ring.set_cursor(0);
            CryptoChain::new(block_ring)
        };

        let mac = snapshots.latest_mac();
        let mut write_buf = WriteBuf::new(CryptoChain::<BlockRing<D>>::AVAIL_BLOCK_SIZE);
        write_buf.write(&Record::Version(mac))?;
        journal_chain.append(write_buf.as_slice())?;
        compaction_policy.on_append_journal(1);
        write_buf.clear();
        journal_chain.flush()?;

        Ok(Self {
            state: init_state,
            journal_chain,
            snapshots,
            compaction_policy,
            curr_edit_group: Some(EditGroup::new()),
            write_buf,
        })
    }

    pub(crate) fn recover(disk: D, meta: &EditJournalMeta, compaction: P) -> Result<Self> {
        let snapshots = SnapshotManager::<S, D>::recover(&disk, meta)?;
        let latest_snapshot_mac = snapshots.latest_mac();
        let latest_snapshot = snapshots.latest_snapshot();
        let mut state = latest_snapshot.state.clone();
        let recover_from = latest_snapshot.recover_from;

        let snapshot_area_offset = meta.snapshot_area_nblocks * 2;
        let block_log =
            disk.subset(snapshot_area_offset..snapshot_area_offset + meta.journal_area_nblocks)?;
        let block_ring = BlockRing::new(block_log);
        let mut recover = CryptoChain::recover(meta.journal_area_key, block_ring, recover_from);

        let mut should_apply = false;
        while let Some(buf) = recover.next() {
            let record_slice = RecordSlice::<E, S>::new(buf);
            for record in record_slice {
                match record {
                    Record::Version(snapshot_mac) => {
                        if snapshot_mac.as_bytes() == latest_snapshot_mac.as_bytes() {
                            should_apply = true;
                        }
                    }
                    Record::Edit(group) => {
                        if should_apply {
                            group.apply_to(&mut state);
                        }
                    }
                }
            }
        }

        let journal_chain = recover.open();
        let new_cursor = journal_chain.block_range().end;
        journal_chain.inner_log().set_cursor(new_cursor);

        Ok(Self {
            state,
            journal_chain,
            snapshots,
            compaction_policy: compaction,
            curr_edit_group: Some(EditGroup::new()),
            write_buf: WriteBuf::new(CryptoChain::<BlockRing<D>>::AVAIL_BLOCK_SIZE),
        })
    }

    pub(crate) fn state(&self) -> &S {
        &self.state
    }

    pub(crate) fn meta(&self) -> EditJournalMeta {
        EditJournalMeta {
            snapshot_area_nblocks: self.snapshots.nblocks(),
            snapshot_area_keys: self.snapshots.keys(),
            journal_area_nblocks: self.journal_chain.inner_log().storage().nblocks(),
            journal_area_key: *self.journal_chain.key(),
        }
    }

    pub(crate) fn add(&mut self, edit: E) {
        let edit_group = self.curr_edit_group.get_or_insert_with(|| EditGroup::new());
        edit_group.push(edit);
    }

    pub(crate) fn commit(&mut self) {
        let Some(edit_group) = self.curr_edit_group.take() else {
            return;
        };
        if edit_group.is_empty() {
            return;
        }

        let record = Record::Edit(edit_group);
        self.write(&record);
        let edit_group = match record {
            Record::Edit(edit_group) => edit_group,
            _ => unreachable!(),
        };
        edit_group.apply_to(&mut self.state);
        self.compaction_policy.on_commit_edits(&edit_group);
    }

    fn write(&mut self, record: &Record<E, S>) {
        let is_first_try_success = self.write_buf.write(record).is_ok();
        if is_first_try_success {
            return;
        }

        self.append_write_buf_to_journal();

        let is_second_try_success = self.write_buf.write(record).is_ok();
        if !is_second_try_success {
            panic!("the write buffer must have enough free space");
        }
    }

    fn append_write_buf_to_journal(&mut self) {
        let write_data = self.write_buf.as_slice();
        if write_data.is_empty() {
            return;
        }

        self.journal_chain
            .append(write_data)
            .expect("we cannot handle I/O error in journaling gracefully");
        self.compaction_policy.on_append_journal(1);
        self.write_buf.clear();

        if self.compaction_policy.should_compact() {
            let compacted_blocks = self.compact().expect("journal chain compaction failed");
            self.compaction_policy.done_compact(compacted_blocks);
        }
    }

    pub(crate) fn flush(&mut self) -> Result<()> {
        self.append_write_buf_to_journal();
        self.journal_chain.flush()
    }

    pub(crate) fn abort(&mut self) {
        if let Some(edits) = self.curr_edit_group.as_mut() {
            edits.clear();
        }
    }

    fn compact(&mut self) -> Result<usize> {
        if self.journal_chain.block_range().is_empty() {
            return Ok(0);
        }

        let latest_snapshot =
            Snapshot::create(self.state().clone(), self.journal_chain.block_range().end);
        self.snapshots.persist(latest_snapshot)?;

        let mac = self.snapshots.latest_mac();
        self.write_buf.write(&Record::Version(mac))?;
        self.journal_chain.append(self.write_buf.as_slice())?;
        self.compaction_policy.on_append_journal(1);
        self.write_buf.clear();

        let old_chain_len = self.journal_chain.block_range().len();
        if old_chain_len > 1 {
            self.journal_chain
                .trim(self.journal_chain.block_range().end - 1);
        }
        let new_chain_len = self.journal_chain.block_range().len();
        Ok(old_chain_len - new_chain_len)
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct Snapshot<S> {
    state: S,
    recover_from: BlockId,
}

impl<S> Snapshot<S> {
    pub(crate) fn create(state: S, recover_from: BlockId) -> Arc<Self> {
        Arc::new(Self {
            state,
            recover_from,
        })
    }

    pub(crate) fn meta_len() -> usize {
        size_of::<BlockId>()
    }
}

struct SnapshotManager<S, D> {
    blobs: [CryptoBlob<D>; 2],
    latest_index: usize,
    buf: Buf,
    snapshot: Arc<Snapshot<S>>,
}

impl<S, D> SnapshotManager<S, D>
where
    S: Serialize + for<'de> Deserialize<'de> + Clone,
    D: BlockSet,
{
    const DEFAULT_LATEST_INDEX: usize = 0;

    pub(crate) fn create(disk: &D, init_state: &S, state_max_nbytes: usize) -> Result<Self> {
        let blob_bytes =
            CryptoBlob::<D>::HEADER_NBYTES + state_max_nbytes + Snapshot::<D>::meta_len();
        let blob_blocks = blob_bytes.div_ceil(BLOCK_SIZE);
        if 2 * blob_blocks >= disk.nblocks() {
            return_errno_with_msg!(OutOfDisk, "the block_set for journal is too small");
        };
        let mut buf = Buf::alloc(blob_blocks)?;

        let snapshot = Snapshot::create(init_state.clone(), 0);
        let serialized = postcard::to_slice(snapshot.as_ref(), buf.as_mut_slice())
            .map_err(|_| Error::with_msg(OutOfDisk, "serialize snapshot failed"))?;

        let block_set0 = disk.subset(0..blob_blocks)?;
        let block_set1 = disk.subset(blob_blocks..blob_blocks * 2)?;
        let blobs = [
            CryptoBlob::create(block_set0, serialized)?,
            CryptoBlob::create(block_set1, serialized)?,
        ];
        Ok(Self {
            blobs,
            latest_index: Self::DEFAULT_LATEST_INDEX,
            buf,
            snapshot,
        })
    }

    pub(crate) fn recover(disk: &D, meta: &EditJournalMeta) -> Result<Self> {
        let mut blob0 = CryptoBlob::open(
            meta.snapshot_area_keys[0],
            disk.subset(0..meta.snapshot_area_nblocks)?,
        );
        let mut blob1 = CryptoBlob::open(
            meta.snapshot_area_keys[1],
            disk.subset(meta.snapshot_area_nblocks..meta.snapshot_area_nblocks * 2)?,
        );

        let mut buf = Buf::alloc(meta.snapshot_area_nblocks)?;
        let snapshot0_res = match blob0.read(buf.as_mut_slice()) {
            Ok(snapshot_len) => {
                postcard::from_bytes::<Snapshot<S>>(&buf.as_slice()[..snapshot_len])
                    .map_err(|_| Error::with_msg(OutOfDisk, "deserialize snapshot0 failed"))
                    .map(Arc::new)
            }
            Err(_) => Err(Error::with_msg(NotFound, "failed to read snapshot0")),
        };
        let snapshot1_res = match blob1.read(buf.as_mut_slice()) {
            Ok(snapshot_len) => {
                postcard::from_bytes::<Snapshot<S>>(&buf.as_slice()[..snapshot_len])
                    .map_err(|_| Error::with_msg(OutOfDisk, "deserialize snapshot1 failed"))
                    .map(Arc::new)
            }
            Err(_) => Err(Error::with_msg(NotFound, "failed to read snapshot1")),
        };

        let snapshots_res = match (snapshot0_res.is_ok(), snapshot1_res.is_ok()) {
            (true, false) => {
                blob1.recover_from(&blob0)?;
                [&snapshot0_res, &snapshot0_res]
            }
            (false, true) => {
                blob0.recover_from(&blob1)?;
                [&snapshot1_res, &snapshot1_res]
            }
            (true, true) => [&snapshot0_res, &snapshot1_res],
            (false, false) => return_errno_with_msg!(
                NotFound,
                "both snapshots are unable to read, recover failed"
            ),
        };

        let version0 = blob0.version_id().unwrap();
        let version1 = blob1.version_id().unwrap();
        let (snapshot_res, latest_index) = match Self::DEFAULT_LATEST_INDEX {
            0 | 1 if version0 == version1 => (
                snapshots_res[Self::DEFAULT_LATEST_INDEX],
                Self::DEFAULT_LATEST_INDEX,
            ),
            0 if version0 + 1 == version1 => (snapshots_res[1], 1),
            1 if version1 + 1 == version0 => (snapshots_res[0], 0),
            _ => return_errno_with_msg!(InvalidArgs, "invalid latest snapshot index or version id"),
        };
        let snapshot = snapshot_res.as_ref().unwrap().clone();
        Ok(Self {
            blobs: [blob0, blob1],
            latest_index,
            buf,
            snapshot,
        })
    }

    pub(crate) fn persist(&mut self, latest: Arc<Snapshot<S>>) -> Result<()> {
        let buf = postcard::to_slice(latest.as_ref(), self.buf.as_mut_slice())
            .map_err(|_| Error::with_msg(OutOfDisk, "serialize current state failed"))?;

        let index = (self.latest_index + 1) % 2;
        self.blobs[index].write(buf)?;
        self.latest_index = index;
        self.snapshot = latest;
        Ok(())
    }

    pub(crate) fn latest_snapshot(&self) -> Arc<Snapshot<S>> {
        self.snapshot.clone()
    }

    pub(crate) fn latest_mac(&self) -> Mac {
        self.blobs[self.latest_index].current_mac().unwrap()
    }

    pub(crate) fn nblocks(&self) -> usize {
        self.blobs[0].nblocks()
    }

    pub(crate) fn keys(&self) -> [Key; 2] {
        [*self.blobs[0].key(), *self.blobs[1].key()]
    }
}

enum Record<E: Edit<S>, S> {
    Version(Mac),
    Edit(EditGroup<E, S>),
}

impl<E: Edit<S>, S> Serialize for Record<E, S> {
    fn serialize<Se>(&self, serializer: Se) -> Result<Se::Ok, Se::Error>
    where
        Se: serde::Serializer,
    {
        match *self {
            Record::Version(ref mac) => {
                serializer.serialize_newtype_variant("Record", 0, "Version", mac)
            }
            Record::Edit(ref edit) => {
                serializer.serialize_newtype_variant("Record", 1, "Edit", edit)
            }
        }
    }
}

impl<'de, E: Edit<S>, S> Deserialize<'de> for Record<E, S> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        enum Variants {
            Version,
            Edit,
        }

        impl<'de> Deserialize<'de> for Variants {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct VariantVisitor;

                impl Visitor<'_> for VariantVisitor {
                    type Value = Variants;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str("`Version` or `Edit`")
                    }

                    fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
                    where
                        E: serde::de::Error,
                    {
                        match v {
                            0 => Ok(Variants::Version),
                            1 => Ok(Variants::Edit),
                            _ => Err(E::custom("unknown value")),
                        }
                    }
                }

                deserializer.deserialize_identifier(VariantVisitor)
            }
        }

        struct RecordVisitor<E: Edit<S>, S> {
            _p: PhantomData<(E, S)>,
        }

        impl<'a, E: Edit<S>, S> Visitor<'a> for RecordVisitor<E, S> {
            type Value = Record<E, S>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a journal record")
            }

            fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::EnumAccess<'a>,
            {
                let (variant, data) = data.variant::<Variants>()?;
                let record = match variant {
                    Variants::Version => {
                        let mac = data.newtype_variant::<Mac>()?;
                        Record::Version(mac)
                    }
                    Variants::Edit => {
                        let edit_group = data.newtype_variant::<EditGroup<E, S>>()?;
                        Record::Edit(edit_group)
                    }
                };
                Ok(record)
            }
        }

        deserializer.deserialize_enum(
            "Record",
            &["Version", "Edit"],
            RecordVisitor { _p: PhantomData },
        )
    }
}

struct WriteBuf<E: Edit<S>, S: Sized> {
    buf: Buf,
    cursor: usize,
    capacity: usize,
    phantom: PhantomData<(E, S)>,
}

impl<E: Edit<S>, S: Sized> WriteBuf<E, S> {
    pub(crate) fn new(capacity: usize) -> Self {
        debug_assert!(capacity <= BLOCK_SIZE);
        Self {
            buf: Buf::alloc(1).unwrap(),
            cursor: 0,
            capacity,
            phantom: PhantomData,
        }
    }

    pub(crate) fn write(&mut self, record: &Record<E, S>) -> Result<()> {
        match postcard::to_slice(record, self.avail_buf()) {
            Ok(serial_record) => {
                self.cursor += serial_record.len();
                Ok(())
            }
            Err(e) => {
                if e != postcard::Error::SerializeBufferFull {
                    panic!(
                        "Errors (except SerializeBufferFull) are not expected: {}",
                        e
                    );
                }
                return_errno_with_msg!(OutOfDisk, "no space for new Record in WriteBuf");
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.buf.as_slice()[..self.cursor]
    }

    fn avail_len(&self) -> usize {
        self.capacity - self.cursor
    }

    fn avail_buf(&mut self) -> &mut [u8] {
        &mut self.buf.as_mut_slice()[self.cursor..self.capacity]
    }
}

struct RecordSlice<'a, E, S> {
    buf: &'a [u8],
    phantom: PhantomData<(E, S)>,
    any_error: bool,
}

impl<'a, E: Edit<S>, S> RecordSlice<'a, E, S> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            phantom: PhantomData,
            any_error: false,
        }
    }

    pub(crate) fn any_error(&self) -> bool {
        self.any_error
    }
}

impl<E: Edit<S>, S> Iterator for RecordSlice<'_, E, S> {
    type Item = Record<E, S>;

    fn next(&mut self) -> Option<Record<E, S>> {
        match postcard::take_from_bytes::<Record<E, S>>(self.buf) {
            Ok((record, left)) => {
                self.buf = left;
                Some(record)
            }
            Err(_) => {
                if !self.buf.is_empty() {
                    self.any_error = true;
                }
                None
            }
        }
    }
}

pub(crate) trait CompactPolicy<E: Edit<S>, S> {
    fn on_commit_edits(&mut self, edits: &EditGroup<E, S>);

    fn on_append_journal(&mut self, appended_blocks: usize);

    fn should_compact(&self) -> bool;

    fn done_compact(&mut self, compacted_blocks: usize);
}

pub(crate) struct NeverCompactPolicy;

impl<E: Edit<S>, S> CompactPolicy<E, S> for NeverCompactPolicy {
    fn on_commit_edits(&mut self, _edits: &EditGroup<E, S>) {}

    fn on_append_journal(&mut self, _appended_nblocks: usize) {}

    fn should_compact(&self) -> bool {
        false
    }

    fn done_compact(&mut self, _compacted_blocks: usize) {}
}

pub(crate) struct DefaultCompactPolicy {
    used_blocks: usize,
    total_blocks: usize,
}

impl DefaultCompactPolicy {
    pub(crate) fn new<D: BlockSet>(disk_nblocks: usize, state_max_nbytes: usize) -> Self {
        let snapshot_bytes =
            CryptoBlob::<D>::HEADER_NBYTES + state_max_nbytes + Snapshot::<D>::meta_len();
        let snapshot_blocks = snapshot_bytes.div_ceil(BLOCK_SIZE);
        debug_assert!(
            snapshot_blocks * 2 < disk_nblocks,
            "the number of blocks of journal area are too small"
        );

        Self {
            used_blocks: 0,
            total_blocks: disk_nblocks - snapshot_blocks * 2,
        }
    }

    pub(crate) fn from_meta(meta: &EditJournalMeta) -> Self {
        Self {
            used_blocks: 0,
            total_blocks: meta.journal_area_nblocks,
        }
    }
}

impl<E: Edit<S>, S> CompactPolicy<E, S> for DefaultCompactPolicy {
    fn on_commit_edits(&mut self, _edits: &EditGroup<E, S>) {}

    fn on_append_journal(&mut self, nblocks: usize) {
        self.used_blocks += nblocks;
    }

    fn should_compact(&self) -> bool {
        self.used_blocks >= self.total_blocks
    }

    fn done_compact(&mut self, compacted_blocks: usize) {
        debug_assert!(self.used_blocks >= compacted_blocks);
        self.used_blocks -= compacted_blocks;
    }
}

#[cfg(test)]
mod tests {
    use ostd_pod::Pod;
    use serde::{Deserialize, Serialize};

    use super::{
        CompactPolicy, DefaultCompactPolicy, Edit, EditGroup, EditJournal, Record, RecordSlice,
        WriteBuf,
    };
    use crate::{
        layers::{
            bio::{BLOCK_SIZE, BlockSet, MemDisk},
            crypto::Mac,
        },
        prelude::*,
    };

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    struct XEdit {
        x: i32,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    struct XState {
        sum: i32,
    }

    impl Edit<XState> for XEdit {
        fn apply_to(&self, state: &mut XState) {
            (*state).sum += self.x;
        }
    }

    struct ThresholdPolicy {
        appended: usize,
        threshold: usize,
    }

    impl ThresholdPolicy {
        pub fn new(threshold: usize) -> Self {
            Self {
                appended: 0,
                threshold,
            }
        }
    }

    impl CompactPolicy<XEdit, XState> for ThresholdPolicy {
        fn on_commit_edits(&mut self, _edits: &EditGroup<XEdit, XState>) {}

        fn on_append_journal(&mut self, nblocks: usize) {
            self.appended += nblocks;
        }

        fn should_compact(&self) -> bool {
            self.appended >= self.threshold
        }

        fn done_compact(&mut self, _compacted_blocks: usize) {
            self.appended = 0;
        }
    }

    #[test]
    fn serde_record() {
        let mut buf = [0u8; 64];
        let mut offset = 0;

        let mut group = EditGroup::<XEdit, XState>::new();
        for x in 0..10 {
            let edit = XEdit { x };
            group.push(edit);
        }
        let mut state = XState { sum: 0 };
        group.apply_to(&mut state);
        let group_len = group.len();
        let edit = Record::<XEdit, XState>::Edit(group);
        let ser = postcard::to_slice(&edit, &mut buf).unwrap();
        println!("serialized edit_group len: {} data: {:?}", ser.len(), ser);
        offset += ser.len();

        let mac = Mac::random();
        let version = Record::<XEdit, XState>::Version(mac);
        let ser = postcard::to_slice(&version, &mut buf[offset..]).unwrap();
        println!("serialize edit_group len: {} data: {:?}", ser.len(), ser);
        offset += ser.len();

        let record_slice = RecordSlice::<XEdit, XState>::new(&buf[..offset]);
        for record in record_slice {
            match record {
                Record::Version(m) => {
                    println!("slice version_mac: {:?}", m);
                    assert_eq!(m.as_bytes(), mac.as_bytes());
                }
                Record::Edit(group) => {
                    println!("slice edit_group len: {}", group.len());
                    assert_eq!(group.len(), group_len);
                }
            }
        }
    }

    #[test]
    fn write_buf() {
        let mut write_buf = WriteBuf::<XEdit, XState>::new(BLOCK_SIZE);

        assert_eq!(write_buf.cursor, 0);
        assert_eq!(write_buf.capacity, BLOCK_SIZE);
        let mut version = 0;
        while write_buf.write(&Record::Version(Mac::random())).is_ok() {
            version += 1;
            let mut group = EditGroup::new();
            for x in 0..version {
                let edit = XEdit { x: x as i32 };
                group.push(edit);
            }
            if write_buf.write(&Record::Edit(group)).is_err() {
                break;
            }
        }
        assert_ne!(write_buf.cursor, 0);

        let record_slice =
            RecordSlice::<XEdit, XState>::new(&write_buf.buf.as_slice()[..write_buf.cursor]);
        let mut version = 0;
        for record in record_slice {
            match record {
                Record::Version(m) => {
                    println!("slice version_mac: {:?}", m);
                    version += 1;
                }
                Record::Edit(group) => {
                    println!("slice edit_group len: {}", group.len());
                    assert_eq!(group.len(), version as usize);
                }
            }
        }
        write_buf.clear();
        assert_eq!(write_buf.cursor, 0);
    }

    fn append_and_recover(threshold: usize, commit_times: usize) {
        let disk = MemDisk::create(64).unwrap();
        let mut journal = EditJournal::format(
            disk.subset(0..16).unwrap(),
            XState { sum: 0 },
            size_of::<XState>() * 2,
            ThresholdPolicy::new(threshold),
        )
        .unwrap();
        let meta = journal.meta();
        assert_eq!(meta.snapshot_area_nblocks, 1);
        assert_eq!(meta.journal_area_nblocks, 14);
        {
            println!("journaling started");
            for _ in 0..commit_times {
                for x in 0..1000 {
                    let edit = XEdit { x };
                    journal.add(edit);
                }
                journal.commit();
                println!("state: {}", journal.state().sum);
            }
        };

        journal.flush().unwrap();

        let journal_disk = disk.subset(0..32).unwrap();
        let threshold_policy = ThresholdPolicy::new(1_000);
        let recover = EditJournal::recover(journal_disk, &meta, threshold_policy).unwrap();
        println!("recover state: {}", recover.state().sum);
        println!(
            "journal chain block range {:?}",
            recover.journal_chain.block_range()
        );
        let append_times = (commit_times - 1) / 2;
        println!("append times: {}", append_times);
        assert_eq!(
            recover.state().sum as usize,
            (0 + 999) * 1000 / 2 * commit_times
        );
        let compact_times = append_times / threshold;
        println!("compact times: {}", compact_times);
    }

    #[test]
    fn edit_journal() {
        append_and_recover(5, 1);
        append_and_recover(5, 10);

        append_and_recover(5, 11);
        append_and_recover(5, 20);

        append_and_recover(5, 21);

        append_and_recover(5, 1000);
    }

    fn default_compact_policy_when_commit(commit_times: usize) {
        let disk = MemDisk::create(16).unwrap();

        let journal_disk = disk.subset(0..12).unwrap();
        let state_max_nbytes = size_of::<XState>() * 2;
        let compact_policy =
            DefaultCompactPolicy::new::<MemDisk>(journal_disk.nblocks(), state_max_nbytes);
        let mut journal: EditJournal<XEdit, XState, MemDisk, DefaultCompactPolicy> =
            EditJournal::format(
                journal_disk,
                XState { sum: 0 },
                state_max_nbytes,
                compact_policy,
            )
            .unwrap();
        let meta = journal.meta();
        assert_eq!(meta.snapshot_area_nblocks, 1);
        assert_eq!(meta.journal_area_nblocks, 10);
        {
            println!("journaling started");
            for _ in 0..commit_times {
                for x in 0..1000 {
                    let edit = XEdit { x };
                    journal.add(edit);
                }
                journal.commit();
                println!("state: {}", journal.state().sum);
            }
        };

        journal.flush().unwrap();

        let journal_disk = disk.subset(0..12).unwrap();
        let compact_policy = DefaultCompactPolicy::from_meta(&meta);
        let recover: EditJournal<XEdit, XState, MemDisk, DefaultCompactPolicy> =
            EditJournal::recover(journal_disk, &meta, compact_policy).unwrap();
        println!("recover state: {}", recover.state().sum);
        assert_eq!(
            recover.state().sum as usize,
            (0 + 999) * 1000 / 2 * commit_times
        );
    }

    #[test]
    fn default_compact_policy() {
        default_compact_policy_when_commit(0);
        default_compact_policy_when_commit(10);
        default_compact_policy_when_commit(100);
        default_compact_policy_when_commit(1000);
    }
}
