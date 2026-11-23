use crate::prelude::*;

pub(crate) const OFFSET_MAX: usize = i64::MAX as usize;

#[derive(Clone, Copy, Debug)]
pub(crate) struct FileRange {
    start: usize,
    end: usize,
}

impl FileRange {
    pub(crate) fn new(start: usize, end: usize) -> Result<Self> {
        if start >= end {
            return Err(errno!(EINVAL, "invalid parameters"));
        }
        Ok(Self { start, end })
    }

    pub(crate) fn len(&self) -> usize {
        self.end - self.start
    }

    pub(crate) fn start(&self) -> usize {
        self.start
    }

    pub(crate) fn end(&self) -> usize {
        self.end
    }

    pub(crate) fn set_start(&mut self, new_start: usize) -> Result<FileRangeChange> {
        if new_start >= self.end {
            return Err(errno!(EINVAL, "invalid new start"));
        }
        let old_start = self.start;
        self.start = new_start;
        let change = match new_start {
            new_start if new_start > old_start => FileRangeChange::Shrunk,
            new_start if new_start < old_start => FileRangeChange::Expanded,
            _ => FileRangeChange::Same,
        };
        Ok(change)
    }

    pub(crate) fn set_end(&mut self, new_end: usize) -> Result<FileRangeChange> {
        if new_end <= self.start {
            return Err(errno!(EINVAL, "invalid new end"));
        }
        let old_end = self.end;
        self.end = new_end;
        let change = match new_end {
            new_end if new_end < old_end => FileRangeChange::Shrunk,
            new_end if new_end > old_end => FileRangeChange::Expanded,
            _ => FileRangeChange::Same,
        };
        Ok(change)
    }

    pub(crate) fn overlap_with(&self, other: &Self) -> Option<OverlapWith> {
        if self.start >= other.end || self.end <= other.start {
            return None;
        }

        let overlap = if self.start <= other.start && self.end < other.end {
            OverlapWith::ToLeft
        } else if self.start > other.start && self.end < other.end {
            OverlapWith::InMiddle
        } else if self.start > other.start && self.end >= other.end {
            OverlapWith::ToRight
        } else {
            OverlapWith::Includes
        };
        Some(overlap)
    }

    pub(crate) fn merge(&mut self, other: &Self) -> Result<FileRangeChange> {
        if self.end < other.start || other.end < self.start {
            return Err(errno!(EINVAL, "can not merge separated ranges"));
        }

        let mut change = FileRangeChange::Same;
        if other.start < self.start {
            self.start = other.start;
            change = FileRangeChange::Expanded;
        }
        if other.end > self.end {
            self.end = other.end;
            change = FileRangeChange::Expanded;
        }
        Ok(change)
    }
}

#[derive(Debug)]
pub(crate) enum FileRangeChange {
    Same,
    Expanded,
    Shrunk,
}

#[derive(Debug)]
pub(crate) enum OverlapWith {
    ToLeft,
    ToRight,
    InMiddle,
    Includes,
}
