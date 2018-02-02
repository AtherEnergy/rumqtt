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
	log.Debugln("Done cleaning ......")
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

func (toxic *Toxic) ZeroUpBandwidth() {
	toxic.Clean()
	log.Debugln("Applying Zero upstream bandwidth Toxic")

	toxic.proxy.AddToxic("bandwidth_up", "bandwidth", "upstream", 1.0, toxiproxy.Attributes{
		"rate": 0,
	})
	toxic.proxy.AddToxic("bandwidth_down", "bandwidth", "downstream", 1.0, toxiproxy.Attributes{
		"rate": 250,
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
		log.Fatalln("Couldn't create proxy client", err)
	}

	toxic := Toxic{client, proxy}

	var clean1 = time.After(1 * time.Second)
	var clean2 <-chan time.Time
	var lowbandwidth <-chan time.Time
	var zeroupbandwidth <-chan time.Time
	var timeout <-chan time.Time
	var halfopen <-chan time.Time

	var toxicTime = 2 * time.Minute

	for {
		select {
		case <-clean1:
			lowbandwidth = time.After(toxicTime)
			toxic.Clean()
		case <-lowbandwidth:
			zeroupbandwidth = time.After(toxicTime)
			toxic.LowBandwidth()
		case <-zeroupbandwidth:
			clean2 = time.After(toxicTime)
			toxic.ZeroUpBandwidth()
		case <-clean2:
			halfopen = time.After(toxicTime)
			toxic.Clean()
		case <-halfopen:
			timeout = time.After(toxicTime)
			toxic.HalfOpenConnection()
		case <-timeout:
			clean1 = time.After(toxicTime)
			toxic.Disconnect()
		}
	}
}
