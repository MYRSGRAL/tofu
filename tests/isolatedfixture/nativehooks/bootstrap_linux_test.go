package nativehooks

import (
	"os"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"testing"
)

func roleRecord() []byte {
	b := make([]byte, 192)
	copy(b, "YNKROLE1")
	b[8] = 1
	b[40] = 2
	be.PutUint64(b[72:], 3)
	b[80] = 4
	be.PutUint64(b[96:], 5)
	b[104] = 6
	be.PutUint32(b[120:], 7)
	be.PutUint64(b[128:], 8)
	be.PutUint16(b[144:], 2)
	be.PutUint16(b[146:], 1)
	be.PutUint16(b[148:], 2)
	return b
}
func TestClosedBootstrap(t *testing.T) {
	b := roleRecord()
	r, e := DecodeBootstrap(b, 2)
	if e != nil || r.ServerPID != 7 || r.Binding.Epoch != 3 {
		t.Fatal(r, e)
	}
	for _, n := range []int{0, 1, 3} {
		if _, e := DecodeBootstrap(b, n); e == nil {
			t.Fatalf("принято %d FD", n)
		}
	}
	for _, n := range []int{0, 136, 137, 143, 144, 146, 148, 150, 191} {
		bad := append([]byte(nil), b...)
		bad[n] ^= 255
		if _, e := DecodeBootstrap(bad, 2); e == nil {
			t.Fatalf("принято изменение byte %d", n)
		}
	}
	for _, n := range []int{0, 136, 191, 193} {
		bad := make([]byte, n)
		copy(bad, b)
		if _, e := DecodeBootstrap(bad, 2); e == nil {
			t.Fatalf("принята длина %d", n)
		}
	}
	if r.CheckServer(nil) == nil {
		t.Fatal("nil pidfd принят")
	}
}

func TestBootstrapRejectsMissingIdentity(t *testing.T) {
	for _, span := range [][2]int{{8, 40}, {40, 72}, {72, 80}, {80, 96}, {96, 104}, {104, 120}, {120, 124}, {128, 136}} {
		b := roleRecord()
		for i := span[0]; i < span[1]; i++ {
			b[i] = 0
		}
		if _, err := DecodeBootstrap(b, 2); err == nil {
			t.Fatalf("принято пустое поле %v", span)
		}
	}
	b := roleRecord()
	be.PutUint32(b[124:], 1000)
	if _, err := DecodeBootstrap(b, 2); err == nil {
		t.Fatal("принят server UID != 0")
	}
}

// Linux pidfd_open имеет номер 434 на поддерживаемых здесь amd64/arm64.
func liveBootstrap(t *testing.T) (Bootstrap, *os.File) {
	t.Helper()
	if runtime.GOARCH != "amd64" && runtime.GOARCH != "arm64" {
		t.Skip("pidfd fixture поддерживает amd64/arm64")
	}
	fd, _, errno := syscall.Syscall(434, uintptr(os.Getpid()), 0, 0)
	if errno != 0 {
		t.Skipf("pidfd_open: %v", errno)
	}
	f := os.NewFile(fd, "test-pidfd")
	t.Cleanup(func() { _ = f.Close() })
	stat, err := os.ReadFile("/proc/self/stat")
	if err != nil {
		t.Fatal(err)
	}
	fields := strings.Fields(string(stat[strings.LastIndexByte(string(stat), ')')+1:]))
	start, err := strconv.ParseUint(fields[19], 10, 64)
	if err != nil {
		t.Fatal(err)
	}
	record := roleRecord()
	be.PutUint32(record[120:], uint32(os.Getpid()))
	be.PutUint64(record[128:], start)
	b, err := DecodeBootstrap(record, 2)
	if err != nil {
		t.Fatal(err)
	}
	return b, f
}
func TestLiveAnchor(t *testing.T) {
	b, f := liveBootstrap(t)
	if err := b.CheckServer(f); err != nil {
		t.Fatal(err)
	}
	b.ServerStarttime++
	if b.CheckServer(f) == nil {
		t.Fatal("принят чужой starttime")
	}
	b.ServerStarttime--
	b.ServerPID++
	if b.CheckServer(f) == nil {
		t.Fatal("принят чужой PID")
	}
}
func TestRecvClosesRejectedRights(t *testing.T) {
	b, pidfd := liveBootstrap(t)
	pair, err := syscall.Socketpair(syscall.AF_UNIX, syscall.SOCK_SEQPACKET|syscall.SOCK_CLOEXEC, 0)
	if err != nil {
		t.Fatal(err)
	}
	receiver := os.NewFile(uintptr(pair[0]), "receiver")
	sender := os.NewFile(uintptr(pair[1]), "sender")
	defer receiver.Close()
	defer sender.Close()
	if err := syscall.SetsockoptInt(pair[0], syscall.SOL_SOCKET, syscall.SO_PASSCRED, 1); err != nil {
		t.Fatal(err)
	}
	pipeR, pipeW, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer pipeR.Close()
	defer pipeW.Close()
	before, err := os.ReadDir("/proc/self/fd")
	if err != nil {
		t.Fatal(err)
	}
	// Некорректный frame с real SCM_RIGHTS обязан закрыть полученный duplicate.
	if err := syscall.Sendmsg(pair[1], []byte("malformed"), syscall.UnixRights(int(pipeR.Fd())), nil, 0); err != nil {
		t.Fatal(err)
	}
	if _, files, err := b.ReceiveAuthenticated(receiver, pidfd); err == nil || len(files) != 0 {
		t.Fatal("принят malformed frame")
	}
	after, err := os.ReadDir("/proc/self/fd")
	if err != nil {
		t.Fatal(err)
	}
	if len(before) != len(after) {
		t.Fatalf("FD leak: %d -> %d", len(before), len(after))
	}
}
