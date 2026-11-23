pub trait Dup: Sized {
    fn dup(&self) -> ostd::Result<Self>;
}
