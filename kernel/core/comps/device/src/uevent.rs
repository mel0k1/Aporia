use alloc::{string::String, vec::Vec};
use core::{
    fmt::{self, Write},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::common::{AnyDevice, Error, Result, node::SysTreeEdit};

const MAX_UEVENT_VARS: usize = 64;
const MAX_UEVENT_BYTES: usize = 2048;
static CURRENT_SEQNUM: AtomicU64 = AtomicU64::new(0);

pub fn emit(dev: &dyn AnyDevice, action: UeventAction) -> Result<()> {
    if !dev.base().is_added() {
        return Err(Error::NotAdded);
    }

    emit_unchecked(dev, action, UeventVars::new())
}

pub(crate) fn emit_unchecked(
    dev: &dyn AnyDevice,
    action: UeventAction,
    extra: UeventVars,
) -> Result<()> {
    let Some(mut builder) = UeventBuilder::from_dev(dev, action)? else {
        return Ok(());
    };

    builder.append_vars(extra)?;
    builder.build()?.commit();

    Ok(())
}

pub fn current_seqnum() -> u64 {
    CURRENT_SEQNUM.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UeventAction {
    Add,
    Remove,
    Change,
    Move,
    Online,
    Offline,
    Bind,
    Unbind,
}

impl UeventAction {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Change => "change",
            Self::Move => "move",
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Bind => "bind",
            Self::Unbind => "unbind",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "add" => Self::Add,
            "remove" => Self::Remove,
            "change" => Self::Change,
            "move" => Self::Move,
            "online" => Self::Online,
            "offline" => Self::Offline,
            "bind" => Self::Bind,
            "unbind" => Self::Unbind,
            _ => return None,
        })
    }
}

impl fmt::Display for UeventAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UeventVars {
    entries: Vec<(String, String)>,
    total_bytes: usize,
}

impl UeventVars {
    pub fn new() -> Self {
        Self::default()
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    pub(crate) fn from_dev(dev: &dyn AnyDevice) -> Result<Self> {
        let mut vars = Self::new();
        if let Some(devnum) = dev.devnum()
            && devnum.id().major().get() != 0
        {
            vars.add("MAJOR", devnum.id().major().get())?;
            vars.add("MINOR", devnum.id().minor().get())?;

            let mode = {
                let node = dev.base().devnode().lock();
                let request = node.as_ref().ok_or(Error::NotAdded)?;
                vars.add("DEVNAME", &request.path)?;
                request.mode.map(|mode| mode & 0o777)
            };
            if let Some(mode) = mode {
                if mode == 0 {
                    vars.add("DEVMODE", "0")?;
                } else {
                    vars.add("DEVMODE", format_args!("0{mode:o}"))?;
                }
            }
        }

        dev.append_uevent_vars(&mut vars)?;

        Ok(vars)
    }

    fn serialize_into(&self, payload: &mut Vec<u8>) {
        for (key, value) in self.iter() {
            payload.extend_from_slice(key.as_bytes());
            payload.push(b'=');
            payload.extend_from_slice(value.as_bytes());
            payload.push(0);
        }
    }

    pub fn add(&mut self, key: &str, value: impl fmt::Display) -> Result<()> {
        if key.is_empty() || key.contains('=') || key.contains('\0') {
            return Err(Error::InvalidValue);
        }

        debug_assert!(self.total_bytes <= MAX_UEVENT_BYTES);
        let key_bytes = key
            .len()
            .checked_add(b"=\0".len())
            .ok_or(Error::MemoryBudgetExhausted)?;
        let limit = (MAX_UEVENT_BYTES - self.total_bytes)
            .checked_sub(key_bytes)
            .ok_or(Error::MemoryBudgetExhausted)?;
        if self.len() == MAX_UEVENT_VARS {
            return Err(Error::MemoryBudgetExhausted);
        }

        let mut writer = ValueWriter::new(limit);
        let result = write!(writer, "{value}");
        if let Some(error) = writer.error {
            return Err(error);
        }
        result?;

        self.total_bytes += key_bytes + writer.value.len();
        self.entries.push((String::from(key), writer.value));

        Ok(())
    }

    fn append(&mut self, mut other: Self) -> Result<()> {
        debug_assert!(self.len() <= MAX_UEVENT_VARS);
        debug_assert!(self.total_bytes <= MAX_UEVENT_BYTES);
        if other.len() > MAX_UEVENT_VARS - self.len()
            || other.total_bytes > MAX_UEVENT_BYTES - self.total_bytes
        {
            return Err(Error::MemoryBudgetExhausted);
        }

        self.total_bytes += other.total_bytes;
        self.entries.append(&mut other.entries);

        Ok(())
    }
}

struct ValueWriter {
    value: String,
    limit: usize,
    error: Option<Error>,
}

impl ValueWriter {
    fn new(limit: usize) -> Self {
        Self {
            value: String::new(),
            limit,
            error: None,
        }
    }
}

impl Write for ValueWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.error.is_some() {
            return Err(fmt::Error);
        }

        if text.contains('\0') {
            self.error = Some(Error::InvalidValue);
            return Err(fmt::Error);
        }

        debug_assert!(self.value.len() <= self.limit);
        if text.len() > self.limit - self.value.len() {
            self.error = Some(Error::MemoryBudgetExhausted);
            return Err(fmt::Error);
        }

        self.value.push_str(text);

        Ok(())
    }
}

pub(crate) struct UeventBuilder {
    action: UeventAction,
    devpath: String,
    subsystem: &'static str,
    vars: UeventVars,
}

impl UeventBuilder {
    pub(crate) fn from_dev(dev: &dyn AnyDevice, action: UeventAction) -> Result<Option<Self>> {
        let subsystem = dev.subsystem();
        let Some(subsystem_name) = subsystem.name() else {
            return Ok(None);
        };

        let devpath = dev.base().tree_path();
        let vars = UeventVars::from_dev(dev)?;

        Ok(Some(Self {
            action,
            devpath,
            subsystem: subsystem_name,
            vars,
        }))
    }

    pub(crate) fn append_vars(&mut self, vars: UeventVars) -> Result<()> {
        self.vars.append(vars)
    }

    pub(crate) fn build(self) -> Result<Uevent> {
        let mut vars = UeventVars::new();
        vars.add("ACTION", self.action.as_str())?;
        vars.add("DEVPATH", &self.devpath)?;
        vars.add("SUBSYSTEM", self.subsystem)?;
        vars.append(self.vars)?;

        let seqnum = CURRENT_SEQNUM
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        vars.add("SEQNUM", seqnum)?;

        let action = self.action.as_str();
        let mut payload = Vec::with_capacity(
            action.len() + self.devpath.len() + b"@\0".len() + vars.total_bytes(),
        );
        payload.extend_from_slice(action.as_bytes());
        payload.push(b'@');
        payload.extend_from_slice(self.devpath.as_bytes());
        payload.push(0);
        vars.serialize_into(&mut payload);

        Ok(Uevent { payload })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Uevent {
    payload: Vec<u8>,
}

impl Uevent {
    pub(crate) fn commit(self) {
        crate::hooks::broadcast_uevent(self.payload)
    }
}

pub(crate) fn parse_synthetic(input: &str) -> Result<(UeventAction, UeventVars)> {
    let request = input
        .strip_suffix('\n')
        .or_else(|| input.strip_suffix('\0'))
        .unwrap_or(input);
    if request.is_empty()
        || request
            .bytes()
            .any(|byte| matches!(byte, b'\0' | b'\n' | b'\r' | b'\t'))
    {
        return Err(Error::InvalidValue);
    }

    let mut parts = request.split(' ');
    let action = parts
        .next()
        .and_then(UeventAction::parse)
        .ok_or(Error::InvalidValue)?;

    let uuid = match parts.next() {
        Some(uuid) if is_valid_uuid(uuid) => uuid,
        Some(_) => return Err(Error::InvalidValue),
        None => "0",
    };

    let mut vars = UeventVars::new();
    vars.add("SYNTH_UUID", uuid)
        .map_err(|_| Error::InvalidValue)?;
    for token in parts {
        let (key, value) = token.split_once('=').ok_or(Error::InvalidValue)?;
        if key.is_empty()
            || value.is_empty()
            || !key.bytes().all(|byte| byte.is_ascii_alphanumeric())
            || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(Error::InvalidValue);
        }

        let synth_key = alloc::format!("SYNTH_ARG_{key}");
        vars.add(&synth_key, value)
            .map_err(|_| Error::InvalidValue)?;
    }

    Ok((action, vars))
}

fn is_valid_uuid(uuid: &str) -> bool {
    uuid.len() == 36
        && uuid.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

pub(crate) fn show(dev: &dyn AnyDevice, writer: &mut dyn Write) -> Result<()> {
    if !dev.base().is_added() {
        return Err(Error::NotAdded);
    }

    let vars = UeventVars::from_dev(dev)?;
    for (key, value) in vars.iter() {
        writeln!(writer, "{key}={value}")?;
    }

    Ok(())
}

pub(crate) fn store(dev: &dyn AnyDevice, text: &str) -> Result<()> {
    if !dev.base().is_added() {
        return Err(Error::NotAdded);
    }

    let (action, vars) = parse_synthetic(text)?;
    emit_unchecked(dev, action, vars)
}
