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
            $self.response_fifo.reset(crate::cd::ZeroFill::Yes);
            push_fifo!($self.response_fifo, $($response,)*);

            $self.interrupts.flags |= $int;

            log::debug!("Set CD-ROM INT{} flag", $int);
        }
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

pub(super) use generate_response;
pub(super) use int2;
pub(super) use int3;
pub(super) use int5;
pub(super) use push_fifo;
