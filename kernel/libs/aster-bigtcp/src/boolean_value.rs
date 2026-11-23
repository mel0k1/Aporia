#[macro_export]
macro_rules! define_boolean_value {
    (
        $(#[$attr:meta])*
        $vis:vis $name: ident
    ) => {
        $(#[$attr])*
        #[derive(Clone, Copy, Debug)]
        $vis struct $name(bool);

        impl $name {
            $vis const TRUE: Self = Self(true);
            $vis const FALSE: Self = Self(false);
        }

        impl core::ops::Deref for $name {
            type Target = bool;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}
