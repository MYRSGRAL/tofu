// Только test overlay: внешний issuer остаётся private Rust fixture.
package nativehooks

import (
	"context"
	"net"
	"os"
	"runtime"
	"sync"
	"syscall"
	"testing"
	"time"
)

func N4Fixture(t *testing.T) (context.Context, func()) {
	t.Helper()
	// Инициализируем netpoll до baseline, чтобы runtime epoll/eventfd не выглядели утечкой.
	warm, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	warm.Close()
	count := func() int {
		files, e := os.ReadDir("/proc/self/fd")
		if e != nil {
			t.Fatal(e)
		}
		return len(files)
	}
	baseline := count()
	t.Cleanup(func() {
		deadline := time.Now().Add(time.Second)
		for count() > baseline {
			if time.Now().After(deadline) {
				t.Fatalf("Go FD lifecycle: %d > %d", count(), baseline)
			}
			runtime.Gosched()
		}
		t.Logf("Go FD lifecycle: %d baseline=%d", count(), baseline)
	})
	fd, err := syscall.Socket(syscall.AF_UNIX, syscall.SOCK_SEQPACKET|syscall.SOCK_CLOEXEC, 0)
	if err != nil {
		t.Fatal(err)
	}
	control := os.NewFile(uintptr(fd), "n4-control")
	t.Cleanup(func() { control.Close() })
	if err = syscall.Connect(fd, &syscall.SockaddrUnix{Name: os.Getenv("YONIK_N4_SOCKET")}); err != nil {
		t.Fatal(err)
	}
	data, ancillary := make([]byte, 192), make([]byte, syscall.CmsgSpace(8))
	n, oob, flags, _, err := syscall.Recvmsg(fd, data, ancillary, syscall.MSG_CMSG_CLOEXEC)
	if err != nil || n != 192 || flags&(syscall.MSG_TRUNC|syscall.MSG_CTRUNC) != 0 {
		t.Fatalf("bootstrap %d %d %v", n, flags, err)
	}
	messages, err := syscall.ParseSocketControlMessage(ancillary[:oob])
	if err != nil || len(messages) != 1 {
		t.Fatal("bootstrap ancillary", err)
	}
	rights, err := syscall.ParseUnixRights(&messages[0])
	if err != nil || len(rights) != 2 {
		t.Fatal("bootstrap roles", err)
	}
	channel, pidfd := os.NewFile(uintptr(rights[0]), "n4-worker"), os.NewFile(uintptr(rights[1]), "n4-pidfd")
	if err = syscall.SetsockoptInt(rights[0], syscall.SOL_SOCKET, syscall.SO_PASSCRED, 1); err != nil {
		t.Fatal(err)
	}
	// READY не даёт HELLO потерять credentials до установки SO_PASSCRED.
	if err = syscall.Sendmsg(fd, []byte("R"), nil, nil, 0); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	result, err := closedAttach(ctx, data, channel, pidfd)
	if os.Getenv("YONIK_N4_MODE") == "wrongmetadata" {
		cancel()
		if err == nil {
			t.Fatal("wrong metadata принята")
		}
		t.Log("wrong metadata: closedAttach отказал до constructor")
		return nil, func() {}
	}
	if err != nil {
		cancel()
		t.Fatal("closedAttach", err)
	}
	c := result.Value(routingKey{}).(*routingConsumer)
	if c.required {
		t.Fatal("fixture не должна выдавать Known proof")
	}
	var once sync.Once
	terminal := func() {
		once.Do(func() {
			if err := syscall.Sendmsg(fd, []byte("T"), nil, nil, 0); err != nil {
				t.Fatal(err)
			}
			select {
			case <-c.session.revokeDone:
			case <-time.After(time.Second):
				t.Fatal("terminal deadline")
			}
			cancel()
		})
	}
	t.Cleanup(func() { terminal(); syscall.Sendmsg(fd, []byte("D"), nil, nil, 0) })
	return result, terminal
}

// N4Link возвращает только проверенную actual ATTACH metadata из fixture context.
func N4Link(ctx context.Context) (string, uint32) {
	c := ctx.Value(routingKey{}).(*routingConsumer)
	return c.attached.Name, c.attached.MTU
}
