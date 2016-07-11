import socket, ssl

def deal_with_client(connstream):
    data = connstream.read()
    # null data means the client is finished with us
    while data:
        if not data:
            # we'll assume do_something returns False
            # when we're finished with client
            break
        print(data)
        data = connstream.read()

key = '/usr/local/Cellar/mosquitto/1.4.4/etc/mosquitto/server_local.key'
crt = '/usr/local/Cellar/mosquitto/1.4.4/etc/mosquitto/server_local.crt'

context = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
context.load_cert_chain(certfile=crt, keyfile=key)

bindsocket = socket.socket()
bindsocket.bind(('localhost', 8883))
bindsocket.listen(5)

while True:
    newsocket, fromaddr = bindsocket.accept()
    print('connection accepted')
    connstream = context.wrap_socket(newsocket, server_side=True)
    try:
        deal_with_client(connstream)
    finally:
        connstream.shutdown(socket.SHUT_RDWR)
        connstream.close()


