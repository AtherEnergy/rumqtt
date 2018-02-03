package main

import (
	"log"
	"math/rand"
	"sync"
	"time"

	mqtt "github.com/eclipse/paho.mqtt.golang"
)

var letters = []rune("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ")

func randSeq(n int) string {
	b := make([]rune, n)
	for i := range b {
		b[i] = letters[rand.Intn(len(letters))]
	}
	return string(b)
}

func main() {
	const TOPIC = "mytopic/test"
	opts := mqtt.NewClientOptions().AddBroker("tcp://localhost:9883").SetKeepAlive(10 * time.Second)

	client := mqtt.NewClient(opts)
	if token := client.Connect(); token.Wait() && token.Error() != nil {
		log.Fatal(token.Error())
	}

	var wg sync.WaitGroup
	wg.Add(1)

	for i := 0; i < 1000; i++ {
		if token := client.Publish(TOPIC, 1, false, randSeq(100)); token.Wait() && token.Error() != nil {
			log.Fatal(token.Error())
		}

		time.Sleep(1 * time.Second)
	}

	wg.Wait()
}
