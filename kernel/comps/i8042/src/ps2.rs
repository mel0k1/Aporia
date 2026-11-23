use crate::controller::{I8042Controller, I8042ControllerError};

const PS2_CMD_RESET: u8 = 0xFF;
const PS2_BAT_OK: u8 = 0xAA;

const PS2_ACK: u8 = 0xFA;
const PS2_NAK: u8 = 0xFE;
const PS2_ERR: u8 = 0xFC;
const PS2_RESULTS: &[u8] = &[PS2_ACK, PS2_NAK, PS2_ERR];

pub(super) trait Command {
    const CMD_BYTE: u8;
    const DATA_LEN: usize;
    const RES_LEN: usize;
}

macro_rules! define_commands {
    (
        $(
            $name:ident, $cmd:literal, fn([u8; $dlen:literal]) -> [u8; $rlen:literal];
        )*
    ) => {
        $(
            pub(super) struct $name;
            impl Command for $name {
                const CMD_BYTE: u8 = $cmd;
                const DATA_LEN: usize = $dlen;
                const RES_LEN: usize = $rlen;
            }
        )*
    };
}
pub(super) use define_commands;

pub(super) trait CommandCtx {
    fn controller(&mut self) -> &mut I8042Controller;

    fn write_to_port(&mut self, data: u8) -> Result<(), I8042ControllerError>;

    fn reset(&mut self) -> Result<Option<u8>, I8042ControllerError> {
        self.write_to_port(PS2_CMD_RESET)?;

        let controller = self.controller();

        if controller.wait_for_specific_data(PS2_RESULTS)? != PS2_ACK {
            return Err(I8042ControllerError::DeviceResetFailed);
        }
        if controller.wait_long_and_recv_data()? != PS2_BAT_OK {
            return Err(I8042ControllerError::DeviceResetFailed);
        }
        Ok(controller.wait_and_recv_data().ok())
    }

    fn command<C: Command>(
        &mut self,
        args: &[u8],
        out: &mut [u8],
    ) -> Result<(), I8042ControllerError> {
        assert_eq!(args.len(), C::DATA_LEN);
        assert_eq!(out.len(), C::RES_LEN);

        self.write_to_port(C::CMD_BYTE)?;
        if self.controller().wait_for_specific_data(PS2_RESULTS)? != PS2_ACK {
            return Err(I8042ControllerError::DeviceResetFailed);
        }

        for &arg in args {
            self.write_to_port(arg)?;
            if self.controller().wait_for_specific_data(PS2_RESULTS)? != PS2_ACK {
                return Err(I8042ControllerError::DeviceResetFailed);
            }
        }

        for slot in out.iter_mut() {
            *slot = self.controller().wait_and_recv_data()?;
        }

        Ok(())
    }
}
