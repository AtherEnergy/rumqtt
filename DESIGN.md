* First connect error should be easily detectable

```
mqtt::run(mqttoptions) -> Result<NotificationReceiver, Error>
```

To report initial connect error, resolve the initial connection future outside thread spawn and move it inside after
the connection is successful