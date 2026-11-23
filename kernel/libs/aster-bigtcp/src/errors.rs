#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindError {
    Exhausted,
    InUse,
}

pub mod tcp {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum IoError<SocketError, CopyError> {
        NoProgress,
        Socket(SocketError),
        Copy(CopyError),
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ListenError {
        InvalidState,
        Unaddressable,
        AddressInUse,
    }

    impl From<smoltcp::socket::tcp::ListenError> for ListenError {
        fn from(value: smoltcp::socket::tcp::ListenError) -> Self {
            match value {
                smoltcp::socket::tcp::ListenError::InvalidState => Self::InvalidState,
                smoltcp::socket::tcp::ListenError::Unaddressable => Self::Unaddressable,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ConnectError {
        InvalidState,
        Unaddressable,
        AddressInUse,
    }

    impl From<smoltcp::socket::tcp::ConnectError> for ConnectError {
        fn from(value: smoltcp::socket::tcp::ConnectError) -> Self {
            match value {
                smoltcp::socket::tcp::ConnectError::InvalidState => Self::InvalidState,
                smoltcp::socket::tcp::ConnectError::Unaddressable => Self::Unaddressable,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SendError {
        InvalidState,
        ConnReset,
    }

    impl From<smoltcp::socket::tcp::SendError> for SendError {
        fn from(value: smoltcp::socket::tcp::SendError) -> Self {
            match value {
                smoltcp::socket::tcp::SendError::InvalidState => Self::InvalidState,
            }
        }
    }

    impl<CopyError> From<smoltcp::socket::tcp::SendError> for IoError<SendError, CopyError> {
        fn from(value: smoltcp::socket::tcp::SendError) -> Self {
            Self::Socket(value.into())
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum RecvError {
        InvalidState,
        Finished,
        ConnReset,
    }

    impl From<smoltcp::socket::tcp::RecvError> for RecvError {
        fn from(value: smoltcp::socket::tcp::RecvError) -> Self {
            match value {
                smoltcp::socket::tcp::RecvError::InvalidState => Self::InvalidState,
                smoltcp::socket::tcp::RecvError::Finished => Self::Finished,
            }
        }
    }

    impl<CopyError> From<smoltcp::socket::tcp::RecvError> for IoError<RecvError, CopyError> {
        fn from(value: smoltcp::socket::tcp::RecvError) -> Self {
            Self::Socket(value.into())
        }
    }
}

pub mod udp {
    pub use smoltcp::socket::udp::RecvError;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SendError {
        Unaddressable,
        BufferFull,
        TooLarge,
    }

    impl From<smoltcp::socket::udp::SendError> for SendError {
        fn from(value: smoltcp::socket::udp::SendError) -> Self {
            match value {
                smoltcp::socket::udp::SendError::Unaddressable => Self::Unaddressable,
                smoltcp::socket::udp::SendError::BufferFull => Self::BufferFull,
            }
        }
    }
}
