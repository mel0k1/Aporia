pub(crate) trait LinuxAbi {
    fn syscall_num(&self) -> usize;

    fn syscall_ret(&self) -> usize;

    fn set_syscall_ret(&mut self, ret: usize);

    fn syscall_args(&self) -> [usize; 6];
}
