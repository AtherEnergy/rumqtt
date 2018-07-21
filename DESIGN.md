##### First connect error should be easily detectable

``` rust
mqtt::run(mqttoptions) -> Result<NotificationReceiver, Error>
```

To report initial connect error, resolve the initial connection future outside thread spawn and move it inside after
the connection is successful

##### User should be able to dynamically reconnect with different configuration

``` rust
    pub enum Request {
        Publish(Publish),
        Subscribe(Subscribe),
        Reconnect(MqttOptions),
        Disconnect,
    }

    client.update_connection(mqttopts);
```

##### Detect halfopen connections. 
    * Halfopen connections can't be detected during read.
    * Write operation won't error out until the tcp write buffer is full
    * Before ping timer sends a ping, it should verify if it has received previous ping response
    * Don't update `last network activity` when tcp writes are successful. \
      They'll be successful when the buffers aren't full even when the network is down.
    * Update network activity based on incoming packets.


