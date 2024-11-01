
#[macro_export]
macro_rules! logln {
    () => {
        println!()
    };
    ($($arg:tt)*) => {{
        println!($($arg)*);
    }};
}