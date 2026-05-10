#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {{
        #[cfg(target_arch = "arm")]
        {
            ::defmt::info!($($arg)*);
        }
        #[cfg(not(target_arch = "arm"))]
        {
            ::std::eprintln!($($arg)*);
        }
    }};
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {{
        #[cfg(target_arch = "arm")]
        {
            ::defmt::warn!($($arg)*);
        }
        #[cfg(not(target_arch = "arm"))]
        {
            ::std::eprintln!($($arg)*);
        }
    }};
}
