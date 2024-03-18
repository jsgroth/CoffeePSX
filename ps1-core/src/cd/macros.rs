macro_rules! push_fifo {
    ($fifo:expr, $($value:expr),* $(,)?) => {
        {
            $(
                $fifo.push($value);
            )*
        }
    }
}

macro_rules! generate_response {
    ($self:expr, $int:expr, $($response:expr),* $(,)?) => {
        {
            $self.response_fifo.reset();
            push_fifo!($self.response_fifo, $($response,)*);

            $self.interrupts.flags |= $int;

            log::debug!("Set CD-ROM INT{} flag", $int);
        }
    }
}

macro_rules! int1 {
    ($self:expr, [$($response:expr),* $(,)?]) => {
        generate_response!($self, 1, $($response,)*);
    }
}

macro_rules! int2 {
    ($self:expr, [$($response:expr),* $(,)?]) => {
        generate_response!($self, 2, $($response,)*);
    }
}

macro_rules! int3 {
    ($self:expr, [$($response:expr),* $(,)?]) => {
        generate_response!($self, 3, $($response,)*);
    }
}

macro_rules! int5 {
    ($self:expr, [$($response:expr),* $(,)?]) => {
        generate_response!($self, 5, $($response,)*);
    }
}

macro_rules! stat {
    ($self:expr, $error_flags:ident) => {
        $self.status_code(crate::cd::status::ErrorFlags::$error_flags)
    };
    ($self:expr) => {
        stat!($self, NONE)
    };
}

pub(super) use generate_response;
pub(super) use int1;
pub(super) use int2;
pub(super) use int3;
pub(super) use int5;
pub(super) use push_fifo;
pub(super) use stat;
