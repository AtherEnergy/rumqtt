import socket
import time, sys
try:
    import ssl
except ImportError:
    pass
else:
    context = ssl.create_default_context()
    context = ssl.SSLContext(ssl.PROTOCOL_SSLv23)
    context.verify_mode = ssl.CERT_REQUIRED
    context.check_hostname = True
    context.load_verify_locations("/Users/ravitejareddy/Downloads/ca.crt")
    conn = context.wrap_socket(socket.socket(socket.AF_INET),
                            server_hostname="localhost")
    conn.connect(("localhost", 8883))

    for i in range(100):
        line = sys.stdin.readline()
        print('sending --> ', line)
        conn.send(line)
    time.sleep(5)
