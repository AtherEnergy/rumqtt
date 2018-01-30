package main

import (
	"time"

	toxiproxy "github.com/Shopify/toxiproxy/client"
	"github.com/sirupsen/logrus"
)

var log = logrus.New()

func init() {
	log.SetLevel(logrus.DebugLevel)
}

type Toxic struct {
	client *toxiproxy.Client
	proxy  *toxiproxy.Proxy
}

func (toxic *Toxic) Clean() {
	log.Debugln("Cleaning Toxics ......")

	toxic.proxy.RemoveToxic("bandwidth_up")
	toxic.proxy.RemoveToxic("bandwidth_down")
	toxic.proxy.RemoveToxic("timeout_up")
}

func (toxic *Toxic) LowBandwidth() {
	toxic.Clean()
	log.Debugln("Applying Bandwidth Toxic. Upstream = 25kbps. Downstream = 25kbps")

	toxic.proxy.AddToxic("bandwidth_up", "bandwidth", "upstream", 1.0, toxiproxy.Attributes{
		"rate": 25,
	})

	toxic.proxy.AddToxic("bandwidth_down", "bandwidth", "downstream", 1.0, toxiproxy.Attributes{
		"rate": 25,
	})
}

func (toxic *Toxic) HalfOpenConnection() {
	toxic.Clean()
	log.Debugln("Applying Halfopen Connection Toxic")

	toxic.proxy.AddToxic("bandwidth_down", "bandwidth", "downstream", 1.0, toxiproxy.Attributes{
		"rate": 0,
	})
}

func (toxic *Toxic) Disconnect() {
	toxic.Clean()
	log.Debugln("Applying Disconnect Timeout Toxic")

	toxic.proxy.AddToxic("timeout_up", "timeout", "upstream", 1.0, toxiproxy.Attributes{
		"timeout": 15,
	})
}

func main() {
	var err error

	// connect to toxiproxy server
	var client = toxiproxy.NewClient("localhost:8474")
	proxy, err := client.CreateProxy("toxicbroker", "127.0.0.1:9883", "127.0.0.1:1883")

	if err != nil {
		log.Fatalln(err)
	}

	toxic := Toxic{client, proxy}

	clean := time.After(1 * time.Second)
	var lowbandwidth <-chan time.Time
	var timeout <-chan time.Time
	var halfopen <-chan time.Time

	for {
		select {
		case <-clean:
			lowbandwidth = time.After(1 * time.Minute)
			toxic.Clean()
		case <-lowbandwidth:
			halfopen = time.After(1 * time.Minute)
			toxic.LowBandwidth()
		case <-halfopen:
			timeout = time.After(1 * time.Minute)
			toxic.HalfOpenConnection()
		case <-timeout:
			clean = time.After(1 * time.Minute)
			toxic.Disconnect()
		}
	}
}
