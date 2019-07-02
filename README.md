[![Build Status](https://dev.azure.com/kraviteza/kraviteza/_apis/build/status/AtherEnergy.rumqtt?branchName=master)](https://dev.azure.com/kraviteza/kraviteza/_build/latest?definitionId=2&branchName=master)
[![Documentation](https://docs.rs/rumqtt/badge.svg)](https://docs.rs/rumqtt)

Pure rust [MQTT] client which strives to be simple, robust and performant. This library takes an opinionated approach of spawning an eventloop thread where all the mqtt related network io happens. The eventloop takes care of necessary robustness tasks without user having to rethink of all the necessary stuff to implement

Client APIs will communicate with the eventloop over a channel. Check the documentation for possible client operations 

#### FEATURES
------------------------------
#### Asynchronous

Asynchronous. New publishs/subscriptions doesn't have to wait for the ack of the previous message before sending the next message. Much faster when compared to synchronous calls

#### Backpressure based on inflight queue length slow networks

Outgoing and incoming streams operating concurrently doesn't mean internal mqtt state buffers grow indefinitely consuming memory. When the inflight message limit is hit, the eventloop stops processing new user requests until inflight queue limit is back to normal and the backpressure will propagate to client calls.

The same is true for slow networks. The channel buffer will nicely smoothen latency spikes but a prolonged bad network will be detected through backpressure.

#### Throttling

A lot of SAAS brokers will allow messages only at a certain rate. Any spikes and the broker might disconnect the client. Throttling limit will make sure that this doesn't happen. This is true even for retransmission of internal unacked messages during reconnections.

#### Automatic reconnections

All the intermittent disconnections are handled with automatic reconnections. But the control to do or not do this is with the user

#### Miscellaneous

* On-demand disconnection and reconnections
* Inbuilt JWT auth for SAAS brokers like GCP iotcore
* Tls using RustTLS. Cross compilation and multi platform support is painless
* Automatic resubscription. Not usually necessary when clean_session=false but might help when opensource brokers crash before saving the state

#### What's not supported

- [ ] Cancelling `mqtt will` with disconnect packet

[MQTT]: http://mqtt.org/