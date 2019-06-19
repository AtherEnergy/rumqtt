[![Build Status](https://travis-ci.org/AtherEnergy/rumqtt.svg)](https://travis-ci.org/AtherEnergy/rumqtt)
[![Documentation](https://docs.rs/rumqtt/badge.svg)](https://docs.rs/rumqtt)

Pure rust [MQTT] client which strives to be simple, robust and performant. This library takes an opnionated approach of spawning an eventloop thread where all the mqtt related network io happens. The eventloop takes care of necessary robustness tasks like

    * automatic reconnections (based on the configuration set by user)
    * throttling on outgoin publishes based on unacked queue sizes
    * jwt token generation for saas iot brokers like gcp iotcore
    * re-subscriptions (based on user configuration)

Client apis will communicate with the eventloop over a channel. Bad networks can be easiliy identified by backpressure in the channel. Checkout the documentaion for possible client operations 

#### What is supported

- [x] QoS 0, 1, 2
- [x] Tls using RustTLS. Cross compilation and multi platform support is painless
- [x] Automatic Reconnection
- [x] Dynamic Reconnection
- [x] Back pressure when the connection is slow
- [x] Incoming notifications
- [x] Pause/Resume network io on demand

#### What's not supported

- [ ] Cancelling `mqtt will` with disconnect packet

[MQTT]: http://mqtt.org/
