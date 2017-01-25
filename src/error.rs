use std::io;

use mqtt3;
use tokio_timer::TimerError;

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Io(err: io::Error) {
            from()
            description("io error")
            display("I/O error: {}", err)
            cause(err)
        }
        Mqtt3(err: mqtt3::Error) {
            from()
        }      
        Timer(err: TimerError) {
            from()
            description("Timer error")
            cause(err)
        }
        Sender {
            description("Sender error")
        }
        Other(descr: &'static str) {
            description(descr)
            display("Error {}", descr)
        }       
        Discard {
            from(&'static str)
        }
    }
}