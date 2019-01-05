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
    * read() call reads data => client knows that the connection is live.
    * Write operation won't error out until the tcp write buffer is full
    * Don't update `last network activity` when tcp writes are successful. \
      They'll be successful when the buffers aren't full even when the network is down.

If pings are sent based on idle reads, client won't send any data to broker when receiving qos0 publishes and the
broker will disconnect the client

If pings are based on idle network replys (because of incoming packets), network activity due to user requests
(like qos1 publishes) will be ignored and spurious pings will be sent

If pings are not triggered because of user requests, which doesn't represent actual outgoing network activity because writes won't error out when the link is down until the buffers are full and internal queues will be filled until writes are blocked/errors because of kernel write buffers being full

pings are sent based on idle user request writes => no pings during user requests resulting in the above problem.

SOLUTION:

Detect bad network based on idle network replys. This makes sure that the client is not fooled by successful publish/subscibe userrequest writes. But this will result in spurious pings when the network is good because userrequest successes are not considered. Trigger pings when there is no outgoing network activity due to incoming packets.

When there are false ping wakeups during active incoming packets but no replys (like user qos1 publishes, connection receives acks but doesn't reply anythin back on network) consider `last_outgoing_time` and chuck the ping.

When there are false ping wakeups when there's no incoming packets and no replys (like user qos0 publishes, connection doesn't receive acks and hence doesn't reply anythin back on network) send a `ping`. Though this ping is unnecessary for broker during good network, it helps client detect halfopen connections during bad network


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





