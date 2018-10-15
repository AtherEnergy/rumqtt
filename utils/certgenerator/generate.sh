#! /bin/bash

# https://jamielinux.com/docs/openssl-certificate-authority/create-the-root-pair.html

set -e


HOME=$PWD
OUT=$PWD/out
META_ROOT=$PWD/meta/root
META_INTERMEDIATE=$PWD/meta/intermediate
COLOR='\033[0;34m'
NC='\033[0m' # No Color

help() {
    echo "Example usage: $0 --client {client_id}"
    echo "Example usage: $0 --server {domain_name}"
    echo
    echo "   --root           | -r                     Delete all the existing certificates and generate new root & intermediate certificates"
    echo "   --server         | -s                     Generate new server certificates using existing intermediate certs"
    echo "   --client         | -c                     Generate new client certificates using existing intermediate certs"
    echo "   --check-root     | -c1                    Check root ca certificate"
    echo "   --check-inter    | -c2                    Check intermedaite ca certificate"
    echo "   --check-server   | -c3                    Check server certificate"
    echo "   --check-client   | -c4                    Check client certificate"
    echo "   --verify-inter   | -v2                    Verify inermediate ca certificate"
    echo "   --verify-server  | -v3                    Verify server certificate"
    echo "   --verify-client  | -v4                    Verify client certificate"
    echo "   --test                                    Instructions for starting test server & client"
    
    echo
    # echo some stuff here for the -a or --add-options 
    exit 1
}

info() {
    echo -e "${COLOR}************************************************************${NC}"
    echo -e "${COLOR}$1${NC}"
    echo -e "${COLOR}$2${NC}"
    echo -e "${COLOR}************************************************************${NC}"
}

gen_root_key_cert() {
    mkdir -p $OUT/root
    cd $OUT/root
    mkdir certs crl newcerts private
    chmod 700 private
    touch index.txt
    echo 1000 > serial

    openssl genrsa -aes256 -out private/ca.key.pem 4096
    chmod 400 private/ca.key.pem

    openssl req -config $META_ROOT/openssl.cnf -key private/ca.key.pem \
            -new -x509 -days 7300 -sha256 -extensions v3_ca -out certs/ca.cert.pem
    chmod 444 certs/ca.cert.pem
    cd $HOME
}

gen_inter_key_cert() {
    mkdir -p $OUT/intermediate
    cd $OUT/intermediate
    mkdir certs crl csr newcerts private
    chmod 700 private
    touch index.txt
    echo 1000 > serial
    echo 1000 > crlnumber


    info "generating intermediate key. set key password"
    openssl genrsa -aes256 -out private/intermediate.key.pem 4096
    chmod 400 private/intermediate.key.pem

    info "creating signing request for intermediate cert generation. \
          \nuse defaults. requires intermediate key password"
    openssl req -config $META_INTERMEDIATE/openssl.cnf -new -sha256 -key private/intermediate.key.pem -out csr/intermediate.csr.pem

    # intermediate ca should be requires root key & config
    info "generating intermediate ca certificate. requires root key password"
    openssl ca -config $META_ROOT/openssl.cnf -extensions v3_intermediate_ca -days 3650 -notext -md sha256 \
               -in csr/intermediate.csr.pem -out certs/intermediate.cert.pem

    chmod 444 certs/intermediate.cert.pem

    info "creating certificate chain file"
    cat certs/intermediate.cert.pem ../root/certs/ca.cert.pem > certs/ca-chain.cert.pem
    chmod 444 certs/ca-chain.cert.pem
    cd $HOME
}

gen_server_cert_key() {
    rm -rf $OUT/server
    mkdir -p $OUT/server
    cd $OUT/server
    mkdir certs crl csr private
    chmod 700 private

    info "generating server key"
    openssl genrsa -out private/server.key.pem 2048
    chmod 400 private/server.key.pem

    info "creating signing request for server cert generation", \
         "NOTE: for server, common name should be fully qualified domain name"

    openssl req -config $META_INTERMEDIATE/openssl.cnf -key private/server.key.pem -new -sha256 -out csr/server.csr.pem \
                -subj "/C=IN/ST=Karnataka/L=Victoria Layout/O=Blah Blah Pvt Ltd/OU=Software Team/CN=$1"

    info "generating server certificate signed with intermediate ca"
    openssl ca -config $META_INTERMEDIATE/openssl.cnf -extensions server_cert -days 375 -notext -md sha256 \
               -in csr/server.csr.pem -out certs/server.cert.pem

    chmod 444 certs/server.cert.pem
    cd $HOME
    instructions
}

gen_client_cert_key() {
    mkdir -p $OUT/client || true
    cd $OUT/client
    mkdir certs crl csr private || true
    chmod 700 private

    info "generating client key"
    openssl genrsa -out private/$1.key.pem 2048
    chmod 400 private/$1.key.pem

    info "creating signing request for client cert generation", \
         "NOTE: for client, common name should be a unique name"

    openssl req -config $META_INTERMEDIATE/openssl.cnf -key private/$1.key.pem -new -sha256 -out csr/$1.csr.pem \
                -subj "/C=IN/ST=Karnataka/L=Victoria Layout/O=Blah Blah Pvt Ltd/OU=Software Team/CN=$1"

    info "generating client certificate signed with intermediate ca"
    openssl ca -config $META_INTERMEDIATE/openssl.cnf -extensions usr_cert -days 375 -notext -md sha256 \
               -in csr/$1.csr.pem -out certs/$1.cert.pem

    chmod 444 certs/$1.cert.pem
    cd $HOME
    instructions
}

validate_server_args () {
    if [ -z "$1" ]; then
        echo -e "\nPlease call '$0 --server {domain_name}' to run this command!\n"
        exit 1
    fi
}

validate_client_args () {
    if [ -z "$1" ]; then
        echo -e "\nThis command requires client id as \n"
        exit 1
    fi
}

instructions () {
    info "use server/certs/server.cert.pem, server/private/server.key.pem, intermediate/certs/ca-chain.cert.pem on server side", \
         "use client/certs/client.cert.pem, client/private/client.key.pem, intermediate/certs/ca-chain.cert.pem on client side" 
}


check_root () {
    cd $OUT/root
    openssl x509 -noout -text -in certs/ca.cert.pem
    cd $HOME
}

check_inter() {
    cd $OUT/intermediate
    openssl x509 -noout -text -in certs/intermediate.cert.pem
    cd $HOME
}

check_server() {
    cd $OUT/server
    openssl x509 -noout -text -in certs/server.cert.pem
    cd $HOME
}

check_client() {
    cd $OUT/client

    openssl x509 -noout -text -in certs/$1.cert.pem
    cd $HOME
}

verify_inter() {
    cd $OUT
    openssl verify -CAfile root/certs/ca.cert.pem intermediate/certs/intermediate.cert.pem
    cd $HOME
}

verify_server() {
    cd $OUT
    openssl verify -CAfile intermediate/certs/ca-chain.cert.pem server/certs/server.cert.pem
    cd $HOME
}

verify_client() {
    cd $OUT
    openssl verify -CAfile intermediate/certs/ca-chain.cert.pem client/certs/$1.cert.pem
    cd $HOME
}

test() {
    # -verify in server makes sure that client sends its certificate
    info "
    copy below command in a seperate terminal
    openssl s_server -accept 12345 -verify \\
                                   -tls1_2 \\
                                   -cert out/server/certs/server.cert.pem \\
                                   -key out/server/private/server.key.pem \\
                                   -CAfile out/intermediate/certs/ca-chain.cert.pem

    NOTE: return status should be 0 and you'll see send text on the server side
    "
    read -p "Press a key after starting the server" choice
    openssl s_client -connect 0.0.0.0:12345 -tls1_2 -cert out/client/certs/$1.cert.pem -key out/client/private/$1.key.pem -CAfile out/intermediate/certs/ca-chain.cert.pem
}

clean ()
{
    rm -rf out
}



#------------------- start processing input arguments ------------------------

if [ $# -eq 0 ]; then
    help
fi

case $1 in
    # generate new client using existing intermediate certificates
    -c|--client)
        validate_client_args $2
        gen_client_cert_key $2
        exit
        ;;

    -s|--server)
        validate_server_args $2
        gen_server_cert_key $2
        exit
        ;;
    # generate new root & intermediate certificates
    -r|--root)
        read -p "Are you sure you want to remove old root & intermediate certs & start from scratch (y/n)?" choice
        case "$choice" in
        y|Y )
            echo "yes"
            clean
            gen_root_key_cert
            gen_inter_key_cert
            exit
            ;;
        n|N )
            echo "exiting"
            exit
            ;;
        * )
            echo "invalid"
            ;;
        esac
        ;;
    -c1|--check-root)
        check_root
        exit
        ;;

    -c2|--check-inter)
        check_inter
        exit
        ;;

    -c3|--check-server)
        check_server
        exit
        ;;

    -c4|--check-client)
        check_client
        exit
        ;;

    -v2|--verify-inter)
        verify_inter
        exit
        ;;

    -v3|--verify-server)
        verify_server
        exit
        ;;

    -v4|--verify-client)
        validate_client_args $2
        verify_client $2
        exit
        ;;
    --test)
        validate_client_args $2
        test $2
        exit
        ;;

    # show help
    * )
        help
        exit
        ;;
esac
