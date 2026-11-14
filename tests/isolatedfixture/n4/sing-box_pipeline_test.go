package nativehooks_test

import (
	"fmt"
	box "github.com/sagernet/sing-box"
	H "github.com/sagernet/sing-box/common/yonikbroker"
	"github.com/sagernet/sing-box/include"
	"github.com/sagernet/sing-box/option"
	"github.com/sagernet/sing/common/json"
	"io"
	"net"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestN4Consumer(t *testing.T) {
	baselineFD, baselineTasks := 0, 0
	var baselineHeap uint64
	for index, mode := range []string{os.Getenv("YONIK_N4_MODE")} {
		t.Run(mode, func(t *testing.T) {
			ctx, revoke := H.N4Fixture(t)
			if ctx == nil {
				return
			}
			name, mtu := H.N4Link(ctx)
			l, err := net.Listen("tcp", "127.0.0.1:0")
			if err != nil {
				t.Fatal(err)
			}
			defer l.Close()
			accepted := make(chan net.Conn, 1)
			writerDone := make(chan error, 1)
			port := l.Addr().(*net.TCPAddr).Port
			u, err := net.ListenPacket("udp", fmt.Sprintf("127.0.0.1:%d", port))
			if err != nil {
				t.Fatal(err)
			}
			defer u.Close()
			go func() {
				for {
					c, e := l.Accept()
					if e != nil {
						return
					}
					go func() {
						defer c.Close()
						accepted <- c
						b := make([]byte, 8)
						if _, e = io.ReadFull(c, b); e != nil {
							return
						}
						if _, e = c.Write(b); e != nil {
							return
						}
						// Большой ответ удерживает writer и packet buffers, пока клиент не читает.
						_, e = c.Write(make([]byte, 32<<20))
						writerDone <- e
					}()
				}
			}()
			go func() {
				b := make([]byte, 2048)
				for {
					n, a, e := u.ReadFrom(b)
					if e != nil {
						return
					}
					u.WriteTo(b[:n], a)
				}
			}()
			ctx = include.Context(ctx)
			var config option.Options
			err = json.UnmarshalContext(ctx, []byte(fmt.Sprintf(`{"inbounds":[{"type":"tun","interface_name":"%s","mtu":%d,"address":["198.18.0.1/24"],"stack":"system","external_configuration":true}],"outbounds":[{"type":"direct","tag":"direct"}],"route":{"rules":[{"action":"route","outbound":"direct","override_address":"127.0.0.1","override_port":%d}]}}`, name, mtu, port)), &config)
			if err != nil {
				t.Fatal(err)
			}
			instance, err := box.New(box.Options{Options: config, Context: ctx})
			if err != nil {
				t.Fatal(err)
			}
			defer instance.Close()

			for _, args := range [][]string{{"addr", "add", "198.18.0.1/24", "dev", name}, {"link", "set", name, "up"}} {
				if out, e := exec.Command("ip", args...).CombinedOutput(); e != nil {
					t.Fatalf("ip: %s %v", out, e)
				}
			}
			if err = instance.Start(); err != nil {
				t.Fatal(err)
			}

			defer exec.Command("ip", "addr", "del", "198.18.0.1/24", "dev", name).Run()
			clients := make([]net.Conn, 0, 2)
			for _, network := range []string{"tcp", "udp"} {
				c, e := net.DialTimeout(network, "198.18.0.2:4242", time.Second)
				if e != nil {
					t.Fatal(e)
				}
				defer c.Close()
				clients = append(clients, c)
				c.SetDeadline(time.Now().Add(2 * time.Second))
				if _, e = c.Write([]byte("pipeline")); e != nil {
					t.Fatal(e)
				}
				b := make([]byte, 8)
				if _, e = io.ReadFull(c, b); e != nil || string(b) != "pipeline" {
					t.Fatalf("%s positive traffic: %q %v", network, b, e)
				}
				t.Logf("%s TUN→routing→outbound echo подтверждён", network)
			}
			peer := <-accepted
			defer peer.Close()
			time.Sleep(30 * time.Millisecond)
			select {
			case <-writerDone:
				t.Fatal("writer не удержан перед terminal")
			default:
			}
			if mode == "stop" {
				if err = instance.Close(); err != nil {
					t.Fatal(err)
				}
			} else {
				revoke()
			}
			select {
			case e := <-writerDone:
				if e == nil {
					t.Fatal("удержанный writer завершился без ошибки")
				}
			case <-time.After(time.Second):
				t.Fatal("outbound writer не завершился после terminal")
			}
			// После terminal новый payload не должен проходить через уже открытый UDP flow.
			c := clients[1]
			c.SetDeadline(time.Now().Add(100 * time.Millisecond))
			c.Write([]byte("afterend"))
			b := make([]byte, 8)
			if _, e := c.Read(b); e == nil {
				t.Fatal("UDP payload после terminal")
			}
			if mode != "stop" {
				revoke()
			}
			deadline := time.Now().Add(time.Second)
			for {
				stack := make([]byte, 1<<20)
				n := runtime.Stack(stack, true)
				text := string(stack[:n])
				active := strings.Contains(text, "route.(*ConnectionManager).connectionCopy") || strings.Contains(text, "proxy/tun.(*stack).read(")
				if !active {
					break
				}
				if time.Now().After(deadline) {
					t.Fatalf("активные core I/O после terminal: %s", text)
				}
				time.Sleep(10 * time.Millisecond)
			}
			t.Log("terminal: удержанный outbound writer завершён; UDP traffic запрещён")

		})
		deadline := time.Now().Add(time.Second)
		for {
			runtime.GC()
			fds, e := os.ReadDir("/proc/self/fd")
			if e != nil {
				t.Fatal(e)
			}
			tasks := runtime.NumGoroutine()
			var memory runtime.MemStats
			runtime.ReadMemStats(&memory)
			if index == 0 {
				baselineHeap = memory.HeapAlloc
				baselineFD = len(fds)
				baselineTasks = tasks
				break
			}
			if len(fds) <= baselineFD && tasks <= baselineTasks && memory.HeapAlloc <= baselineHeap+(8<<20) {
				t.Logf("terminal fd=%d tasks=%d baseline=%d/%d", len(fds), tasks, baselineFD, baselineTasks)
				break
			}
			if time.Now().After(deadline) {
				stack := make([]byte, 1<<20)
				n := runtime.Stack(stack, true)
				t.Logf("%s", stack[:n])
				for _, fd := range fds {
					target, _ := os.Readlink("/proc/self/fd/" + fd.Name())
					t.Logf("fd %s %s", fd.Name(), target)
				}
				t.Fatalf("resource growth: fd=%d/%d tasks=%d/%d heap=%d/%d", len(fds), baselineFD, tasks, baselineTasks, memory.HeapAlloc, baselineHeap)
			}
			time.Sleep(10 * time.Millisecond)
		}
	}
}
