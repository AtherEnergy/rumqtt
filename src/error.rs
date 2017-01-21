use std::io;

use mqtt::topic_name::TopicNameError;
use mqtt3;

error_chain! {
    foreign_links {
        Io(io::Error);
        TopicName(TopicNameError);
        Mqtt3(mqtt3::Error);
    }

    errors {
        // MQTT(e: mqtt3::Error) { description("MQTT error") display("MQTT says: {:?}", e) }
    }
 }

//  impl From<mqtt3::Error> for io::Error {
//     fn from(err: mqtt3::Error) -> io::Error {
//         // match err {
//         //     Error::Io(e) => e,
//         //     _ => panic!("invalid error"),
//         // }
//         panic!("hello world");
//     }
// }
