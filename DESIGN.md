##### First connect error should be easily detectable

``` rust
mqtt::run(mqttoptions) -> Result<NotificationReceiver, Error>
```

Since the reactor which's doing network operations runs in its own thread, returning results to the user
becomes tricky. But the user should have an option to know the status of initial connection through API
rather than the log. The result of the connection is passed to the user via channel as we can't
move reactor between threads. This is also a reason behind the separation of connection and mqttio futures
and running them separately on the reactor

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

##### Strategy to Detect halfopen connections. 
    * Halfopen connections can't be detected during idle read.
    * But if the read() call reads data implies the connection is live.
    * Write operation won't error out until the tcp write buffer is full
    * Don't update `last network activity` when tcp writes are successful. \
      They'll be successful when the buffers aren't full even when the network is down.

Detect half open connections at broker end:

Broker needs to know periodically if the connection is alive or else it'll disconnect the client to get
rid of half open connections. If there are no outgoing packets, send a PINGREQ.

Detect half open connections at client end:

Checking the status of connection is not possible through successful tcp writes becuase of buffering.
Send a PINGREQ and wait for PINGRESP. Check for previous PINGRESP before sending a PINGREQ and disconnect
if not received. We can get rid of halfopen connections during second ping

##### Play pause

Sometimes we need to ask the client to stop network operations so that other high priority applications
load/send their data faster.

```
  select! {
    request_channel, command_channel
  }
```

disconnect from the network when command channel sends `Pause` command and create a stream which only
listens on the `command_channel` during next iteration.





