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
    * Halfopen connections can't be detected during idle read.
    * But if the read() call reads data implies the connection is live.
    * Update network activity with read calls and only ping when necessary.
    * Write operation won't error out until the tcp write buffer is full
    * Don't update `last network activity` when tcp writes are successful. \
      They'll be successful when the buffers aren't full even when the network is down.
    * Finally to dectect half open connections, ping when there is no network activity and
      the next ping should verify previous ping response and disconnect if it didn't receive any



