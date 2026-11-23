use core::{mem::offset_of, time::Duration};

use aster_util::{field_ptr, safe_ptr::SafePtr};
use device_id::{DeviceId, MinorId};
use ostd::{
    const_assert,
    mm::{FrameAllocOptions, HasPaddr, HasSize, USegment, VmIo, dma::DmaCoherent},
    sync::{RwMutexWriteGuard, Waiter},
};
use spin::Once;
use tdx_guest::{
    tdcall::{self, TdCallError},
    tdvmcall::{self, TdVmcallError},
};

use crate::{
    device::{Device, DeviceType, registry::char::register},
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, StatusFlags},
        vfs::{inode::FileOps, path::Path},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

const TDX_GUEST_MINOR: u32 = 0x7b;

#[derive(Debug)]
pub(crate) struct TdxGuest {
    id: DeviceId,
}

impl TdxGuest {
    pub(crate) fn new() -> Arc<Self> {
        let major = super::MISC_MAJOR.get().unwrap().get();
        let minor = MinorId::new(TDX_GUEST_MINOR);

        let id = DeviceId::new(major, minor);
        Arc::new(Self { id })
    }
}

impl Device for TdxGuest {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        Some(DevtmpfsNodeMeta::new("tdx_guest").unwrap())
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(TdxGuestFile))
    }
}

impl From<TdCallError> for Error {
    #[track_caller]
    fn from(err: TdCallError) -> Self {
        match err {
            TdCallError::TdxNoValidVeInfo => {
                errno!(EINVAL, "TdCallError::TdxNoValidVeInfo")
            }
            TdCallError::TdxOperandInvalid => {
                errno!(EINVAL, "TdCallError::TdxOperandInvalid")
            }
            TdCallError::TdxPageAlreadyAccepted => {
                errno!(EINVAL, "TdCallError::TdxPageAlreadyAccepted")
            }
            TdCallError::TdxPageSizeMismatch => {
                errno!(EINVAL, "TdCallError::TdxPageSizeMismatch")
            }
            TdCallError::TdxOperandBusy => {
                errno!(EBUSY, "TdCallError::TdxOperandBusy")
            }
            TdCallError::Other => errno!(EAGAIN, "TdCallError::Other"),
            _ => todo!(),
        }
    }
}

impl From<TdVmcallError> for Error {
    #[track_caller]
    fn from(err: TdVmcallError) -> Self {
        match err {
            TdVmcallError::TdxRetry => {
                errno!(EINVAL, "TdVmcallError::TdxRetry")
            }
            TdVmcallError::TdxOperandInvalid => {
                errno!(EINVAL, "TdVmcallError::TdxOperandInvalid")
            }
            TdVmcallError::TdxGpaInuse => {
                errno!(EINVAL, "TdVmcallError::TdxGpaInuse")
            }
            TdVmcallError::TdxAlignError => {
                errno!(EINVAL, "TdVmcallError::TdxAlignError")
            }
            TdVmcallError::Other => errno!(EAGAIN, "TdVmcallError::Other"),
        }
    }
}

struct TdxGuestFile;

impl Pollable for TdxGuestFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileOps for TdxGuestFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(errno!(EINVAL, "the file is not valid for reading"))
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(errno!(EINVAL, "the file not valid for writing"))
    }
}

impl PerOpenFileOps for TdxGuestFile {
    fn check_seekable(&self) -> Result<()> {
        Err(errno!(ESPIPE, "seek is not supported"))
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, _path: &Path, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetTdxReport => {
                cmd.with_data_ptr(|data_ptr| {
                    let inblob = {
                        let inblob_ptr = field_ptr!(&data_ptr, TdxReportRequest, report_data);
                        inblob_ptr.read()?
                    };

                    let report = tdx_report_or_err()?.write();
                    refresh_tdx_report_locked(&report, Some(inblob.as_bytes()))?;
                    let outblob_ptr = field_ptr!(&data_ptr, TdxReportRequest, tdx_report);
                    outblob_ptr.copy_from(&SafePtr::new(&*report, 0))?;

                    Ok(0)
                })
            }
            _ => Err(errno!(ENOTTY, "the ioctl command is unknown")),
        })
    }
}

static TDX_REPORT: Once<RwMutex<USegment>> = Once::new();

fn tdx_report_or_err() -> Result<&'static RwMutex<USegment>> {
    TDX_REPORT
        .get()
        .ok_or_else(|| errno!(ENODEV, "the TDX report is not initialized"))
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Rtmr {
    Rtmr0,
    Rtmr1,
    Rtmr2,
    Rtmr3,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MeasurementReg {
    MrConfigId,
    MrOwner,
    MrOwnerConfig,
    MrTd,
    Rtmr(Rtmr),
}

pub(crate) fn tdx_get_quote(inblob: &[u8]) -> Result<Box<[u8]>> {
    const GET_QUOTE_IN_FLIGHT: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    const GET_QUOTE_BUF_SIZE: usize = 8 * 1024;

    let buf = DmaCoherent::alloc(GET_QUOTE_BUF_SIZE.div_ceil(PAGE_SIZE), false)?;
    let header_ptr: SafePtr<TdxQuoteHdr, _, _> = SafePtr::new(&buf, 0);
    let payload_ptr: SafePtr<TdReport, _, _> = SafePtr::new(&buf, size_of::<TdxQuoteHdr>());

    field_ptr!(&header_ptr, TdxQuoteHdr, version).write(&1u64)?;
    field_ptr!(&header_ptr, TdxQuoteHdr, status).write(&0u64)?;
    field_ptr!(&header_ptr, TdxQuoteHdr, in_len).write(&(size_of::<TdReport>() as u32))?;
    field_ptr!(&header_ptr, TdxQuoteHdr, out_len).write(&0u32)?;

    let report = tdx_report_or_err()?.write();
    refresh_tdx_report_locked(&report, Some(inblob))?;
    payload_ptr.copy_from(&SafePtr::new(&*report, 0))?;
    drop(report);

    let shared_mask = tdx_guest::shared_mask();
    tdvmcall::get_quote((buf.paddr() as u64) | shared_mask, buf.size() as u64)?;

    let status_ptr = field_ptr!(&header_ptr, TdxQuoteHdr, status);
    let (sleep_waiter, _) = Waiter::new_pair();
    let sleep_duration = Duration::from_millis(100);
    loop {
        let status = status_ptr.read()?;
        if status != GET_QUOTE_IN_FLIGHT {
            break;
        }
        let _ = sleep_waiter.wait_until_or_timeout(|| -> Option<()> { None }, &sleep_duration);
    }

    let out_len = field_ptr!(&header_ptr, TdxQuoteHdr, out_len).read()?;
    let mut outblob = vec![0u8; out_len as usize].into_boxed_slice();
    payload_ptr.cast().read_slice(outblob.as_mut())?;
    Ok(outblob)
}

pub(crate) fn refresh_tdx_report(inblob: Option<&[u8]>) -> Result<()> {
    let report = tdx_report_or_err()?.write();
    refresh_tdx_report_locked(&report, inblob)
}

pub(crate) const SHA384_DIGEST_SIZE: usize = 48;

pub(crate) fn get_tdx_mr(reg: MeasurementReg) -> Result<[u8; SHA384_DIGEST_SIZE]> {
    let report = tdx_report_or_err()?.read();

    let mut blob = [0u8; SHA384_DIGEST_SIZE];
    report
        .read_bytes(TdReport::mr_offset(reg), blob.as_mut())
        .unwrap();
    Ok(blob)
}

pub(crate) fn get_tdx_mr_refresh(reg: MeasurementReg) -> Result<[u8; SHA384_DIGEST_SIZE]> {
    let report = tdx_report_or_err()?.write();

    refresh_tdx_report_locked(&report, None)?;

    let mut blob = [0u8; SHA384_DIGEST_SIZE];
    report
        .read_bytes(TdReport::mr_offset(reg), blob.as_mut())
        .unwrap();
    Ok(blob)
}

pub(crate) fn extend_tdx_mr(reg: Rtmr, data: &[u8; SHA384_DIGEST_SIZE]) -> Result<()> {
    let index = match reg {
        Rtmr::Rtmr0 => 0,
        Rtmr::Rtmr1 => 1,
        Rtmr::Rtmr2 => 2,
        Rtmr::Rtmr3 => 3,
    };

    let buf: USegment = FrameAllocOptions::new().alloc_segment(1)?.into();
    buf.write_bytes(0, data).unwrap();

    tdcall::extend_rtmr(buf.paddr() as u64, index)?;
    Ok(())
}

pub(super) fn init() -> Result<()> {
    TDX_REPORT.call_once(|| {
        let report = FrameAllocOptions::new()
            .alloc_segment(size_of::<TdReport>().div_ceil(PAGE_SIZE))
            .unwrap()
            .into();
        RwMutex::new(report)
    });
    refresh_tdx_report(None)?;
    register(TdxGuest::new())?;
    Ok(())
}

fn refresh_tdx_report_locked(
    report: &RwMutexWriteGuard<'_, USegment>,
    inblob: Option<&[u8]>,
) -> Result<()> {
    if let Some(inblob) = inblob {
        if inblob.len() != size_of::<ReportData>() {
            return Err(errno!(EINVAL, "invalid inblob length"));
        }

        report
            .write_bytes(TdReport::report_data_offset(), inblob)
            .unwrap();
    }

    let report_data_ptr = report.paddr() + TdReport::report_data_offset();

    tdcall::get_report(report.paddr() as u64, report_data_ptr as u64)?;
    Ok(())
}

#[repr(C)]
struct TdxQuoteHdr {
    version: u64,
    status: u64,
    in_len: u32,
    out_len: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct TdxReportRequest {
    report_data: ReportData,
    tdx_report: TdReport,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct ReportData {
    data: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct TdReport {
    report_mac: ReportMac,
    _reserved: [u8; 256],
    td_info: TdInfo,
}
const_assert!(size_of::<TdReport>() == 1024);

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct ReportMac {
    _reserved1: [u8; 128],
    report_data: ReportData,
    _reserved2: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct TdInfo {
    attributes: [u8; 8],
    xfam: [u8; 8],
    mrtd: [u8; 48],
    mrconfigid: [u8; 48],
    mrowner: [u8; 48],
    mrownerconfig: [u8; 48],
    rtmr0: [u8; 48],
    rtmr1: [u8; 48],
    rtmr2: [u8; 48],
    rtmr3: [u8; 48],
    servtd_hash: [u8; 48],
    extension: [u8; 64],
}

impl TdReport {
    const fn report_data_offset() -> usize {
        offset_of!(TdReport, report_mac) + offset_of!(ReportMac, report_data)
    }

    const fn mr_offset(reg: MeasurementReg) -> usize {
        offset_of!(TdReport, td_info)
            + match reg {
                MeasurementReg::MrConfigId => offset_of!(TdInfo, mrconfigid),
                MeasurementReg::MrOwner => offset_of!(TdInfo, mrowner),
                MeasurementReg::MrOwnerConfig => offset_of!(TdInfo, mrownerconfig),
                MeasurementReg::MrTd => offset_of!(TdInfo, mrtd),
                MeasurementReg::Rtmr(Rtmr::Rtmr0) => offset_of!(TdInfo, rtmr0),
                MeasurementReg::Rtmr(Rtmr::Rtmr1) => offset_of!(TdInfo, rtmr1),
                MeasurementReg::Rtmr(Rtmr::Rtmr2) => offset_of!(TdInfo, rtmr2),
                MeasurementReg::Rtmr(Rtmr::Rtmr3) => offset_of!(TdInfo, rtmr3),
            }
    }
}

mod ioctl_defs {
    use super::TdxReportRequest;
    use crate::{ioc, util::ioctl::InOutData};

    pub(super) type GetTdxReport =
        ioc!(TDX_CMD_GET_REPORT0, b'T', 0x01, InOutData<TdxReportRequest>);
}
