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

	toxic.proxy.UpdateToxic("bandwidth_up", 1.0, toxiproxy.Attributes{
		"rate": 500,
	})

	toxic.proxy.UpdateToxic("bandwidth_down", 1.0, toxiproxy.Attributes{
		"rate": 500,
	})

	toxic.proxy.RemoveToxic("timeout_up")
	log.Debugln("Done cleaning ......")
}

func (toxic *Toxic) LowBandwidth() {
	toxic.Clean()
	log.Debugln("Applying Bandwidth Toxic. Upstream = 25kbps. Downstream = 25kbps")

	toxic.proxy.UpdateToxic("bandwidth_up", 1.0, toxiproxy.Attributes{
		"rate": 25,
	})

	toxic.proxy.UpdateToxic("bandwidth_down", 1.0, toxiproxy.Attributes{
		"rate": 25,
	})
}

func (toxic *Toxic) ZeroUpBandwidth() {
	toxic.Clean()
	log.Debugln("Applying Zero upstream bandwidth Toxic")

	toxic.proxy.UpdateToxic("bandwidth_up", 1.0, toxiproxy.Attributes{
		"rate": 0,
	})

	toxic.proxy.UpdateToxic("bandwidth_down", 1.0, toxiproxy.Attributes{
		"rate": 500,
	})
}

func (toxic *Toxic) HalfOpenConnection() {
	toxic.Clean()
	log.Debugln("Applying Halfopen Connection Toxic")

	toxic.proxy.UpdateToxic("bandwidth_down", 1.0, toxiproxy.Attributes{
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
		log.Fatalln("Couldn't create proxy client", err)
	}

	toxic := Toxic{client, proxy}
	delay := 1 * time.Minute

	toxic.proxy.AddToxic("bandwidth_up", "bandwidth", "upstream", 1.0, toxiproxy.Attributes{
		"rate": 500,
	})

	toxic.proxy.AddToxic("bandwidth_down", "bandwidth", "downstream", 1.0, toxiproxy.Attributes{
		"rate": 500,
	})

	for {
		toxic.Clean()
		time.Sleep(delay)

		toxic.LowBandwidth()
		time.Sleep(delay)

		toxic.ZeroUpBandwidth()
		time.Sleep(delay)

		toxic.Clean()
		time.Sleep(delay)

		toxic.HalfOpenConnection()
		time.Sleep(delay)

		toxic.Clean()
		time.Sleep(delay)

		toxic.Disconnect()
		time.Sleep(delay)
	}
}
