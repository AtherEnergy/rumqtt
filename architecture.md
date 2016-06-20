
* `client options` create `subsciber`, `publisher` and `proxy`

* `subscriber` & `publisher` are just user interfaces to pass requests (through channels) to `proxy`

* `proxy` processes client requests and handles network events

* `proxy` CREATES a `proxy client` to do most of the handling. all the mqtt state + queues are held in `proxy client`

* `proxy` carries some necessary initial state from `client options` to pass to `proxy client`

* but all the CHANNEL functionality is held in `proxy` itself (better functional seggregation and no borrow checker woes when trying to use `channels` and `proxy client` at the same time)