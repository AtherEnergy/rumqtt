// use super::client::MqttClient;
// use std::sync::{Arc, Mutex};

// impl MqttClient {

//     pub fn on_message<F>(&self, callback: F) -> Result<&Self, i32>
//         where F: Fn(&str, &str) + Send + Sync + 'static
//     {
//         let callback = Arc::new(Mutex::new(callback));

//         match self.msg_callback {
//             Some(ref t) => t.send(callback),
//             None => return Err(-10),
//         };

//         Ok(self)
//     }
// }
