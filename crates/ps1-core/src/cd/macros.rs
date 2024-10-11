macro_rules! stat {
    ($self:expr, $error_flags:ident) => {
        $self.status_code(crate::cd::status::ErrorFlags::$error_flags)
    };
    ($self:expr) => {
        stat!($self, NONE)
    };
}

pub(super) use stat;
