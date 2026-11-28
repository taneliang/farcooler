// Local TURN server for the ov-1502 spike (UDP and TCP on 127.0.0.1:3478, user spike/spike).
package main

import (
	"log"
	"net"
	"os"
	"os/signal"

	"github.com/pion/turn/v4"
)

func main() {
	udp, err := net.ListenPacket("udp4", "127.0.0.1:3478")
	if err != nil { log.Fatal(err) }
	tcp, err := net.Listen("tcp4", "127.0.0.1:3478")
	if err != nil { log.Fatal(err) }
	key := turn.GenerateAuthKey("spike", "spike.local", "spike")
	gen := &turn.RelayAddressGeneratorStatic{RelayAddress: net.ParseIP("127.0.0.1"), Address: "127.0.0.1"}
	s, err := turn.NewServer(turn.ServerConfig{
		Realm: "spike.local",
		AuthHandler: func(u, realm string, src net.Addr) ([]byte, bool) {
			log.Printf("auth %s from %s", u, src)
			return key, u == "spike"
		},
		PacketConnConfigs: []turn.PacketConnConfig{{PacketConn: udp, RelayAddressGenerator: gen}},
		ListenerConfigs:   []turn.ListenerConfig{{Listener: tcp, RelayAddressGenerator: gen}},
	})
	if err != nil { log.Fatal(err) }
	log.Print("TURN on 127.0.0.1:3478 udp+tcp")
	c := make(chan os.Signal, 1); signal.Notify(c, os.Interrupt); <-c; s.Close()
}
