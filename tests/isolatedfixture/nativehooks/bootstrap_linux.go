package nativehooks

import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// Bootstrap содержит только поля замороженного closed worker handoff.
// Вызывающий protected worker обязан подтвердить происхождение record до exec.
// Разбор произвольной строки окружения сам по себе authority не создаёт.
type Bootstrap struct {
	Binding         Binding
	Session         [16]byte
	Generation      uint64
	Invocation      [16]byte
	ServerPID       uint32
	ServerStarttime uint64
}

func DecodeBootstrap(b []byte, fdCount int) (Bootstrap, error) {
	var r Bootstrap
	if len(b) != 192 || fdCount != 2 || string(b[:8]) != "YNKROLE1" || zero(b[8:40]) || zero(b[40:72]) || u64(b, 72) == 0 || zero(b[80:96]) || u64(b, 96) == 0 || zero(b[104:120]) || u32(b, 120) == 0 || u32(b, 124) != 0 || u64(b, 128) == 0 || !zero(b[136:144]) || u16(b, 144) != 2 || u16(b, 146) != 1 || u16(b, 148) != 2 || !zero(b[150:192]) {
		return r, ErrMalformed
	}
	copy(r.Binding.Nonce[:], b[8:40])
	copy(r.Binding.ConfigDigest[:], b[40:72])
	r.Binding.Epoch = u64(b, 72)
	copy(r.Session[:], b[80:96])
	r.Generation = u64(b, 96)
	copy(r.Invocation[:], b[104:120])
	r.ServerPID = u32(b, 120)
	r.ServerStarttime = u64(b, 128)
	return r, nil
}

// CheckServer сверяет live pidfd и starttime. FD принадлежит custody вызывающего.
// pidfd readable при exit, поэтому одного существующего /proc/PID недостаточно.
func (b Bootstrap) CheckServer(pidfd *os.File) error {
	if pidfd == nil {
		return ErrDenied
	}
	fd := int(pidfd.Fd())
	var set syscall.FdSet
	if fd < 0 || fd >= len(set.Bits)*64 {
		return ErrDenied
	}
	set.Bits[fd/64] |= 1 << uint(fd%64)
	timeout := syscall.Timeval{}
	n, err := syscall.Select(fd+1, &set, nil, nil, &timeout)
	if err != nil || n != 0 {
		return ErrDenied
	}
	info, err := os.ReadFile(fmt.Sprintf("/proc/self/fdinfo/%d", fd))
	if err != nil {
		return ErrDenied
	}
	found := false
	for _, line := range strings.Split(string(info), "\n") {
		fields := strings.Fields(line)
		if len(fields) == 2 && fields[0] == "Pid:" {
			pid, e := strconv.ParseUint(fields[1], 10, 32)
			if e != nil || pid != uint64(b.ServerPID) {
				return ErrDenied
			}
			found = true
		}
	}
	if !found {
		return ErrDenied
	}
	stat, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", b.ServerPID))
	if err != nil {
		return ErrDenied
	}
	end := strings.LastIndexByte(string(stat), ')')
	if end < 0 {
		return ErrDenied
	}
	fields := strings.Fields(string(stat[end+1:]))
	if len(fields) <= 19 {
		return ErrDenied
	}
	start, err := strconv.ParseUint(fields[19], 10, 64)
	if err != nil || start != b.ServerStarttime {
		return ErrDenied
	}
	return nil
}

// ReceiveAuthenticated принимает владение всеми полученными FD даже при отказе.
// Возвращённые FD переходят вызывающему только после всех проверок.
func (b Bootstrap) ReceiveAuthenticated(channel, pidfd *os.File) (Frame, []*os.File, error) {
	fail := func(files []*os.File) (Frame, []*os.File, error) {
		for _, f := range files {
			_ = f.Close()
		}
		return Frame{}, nil, ErrDenied
	}
	if channel == nil || b.CheckServer(pidfd) != nil {
		return fail(nil)
	}
	kind, err := syscall.GetsockoptInt(int(channel.Fd()), syscall.SOL_SOCKET, syscall.SO_TYPE)
	if err != nil || kind != syscall.SOCK_SEQPACKET {
		return fail(nil)
	}
	pass, err := syscall.GetsockoptInt(int(channel.Fd()), syscall.SOL_SOCKET, syscall.SO_PASSCRED)
	if err != nil || pass != 1 {
		return fail(nil)
	}
	data := make([]byte, MaxFrameBytes+1)
	control := make([]byte, syscall.CmsgSpace(4*16)+syscall.CmsgSpace(syscall.SizeofUcred))
	fd := int(channel.Fd())
	var ready syscall.FdSet
	if fd < 0 || fd >= len(ready.Bits)*64 {
		return fail(nil)
	}
	ready.Bits[fd/64] |= 1 << uint(fd%64)
	timeout := syscall.NsecToTimeval(int64(20 * time.Millisecond))
	count, waitErr := syscall.Select(fd+1, &ready, nil, nil, &timeout)
	if waitErr != nil || count != 1 {
		return fail(nil)
	}
	n, oobn, flags, _, err := syscall.Recvmsg(fd, data, control, syscall.MSG_CMSG_CLOEXEC|syscall.MSG_DONTWAIT)
	var files []*os.File
	messages, parseErr := syscall.ParseSocketControlMessage(control[:oobn])
	credentials := 0
	valid := true
	for _, message := range messages {
		switch {
		case message.Header.Level == syscall.SOL_SOCKET && message.Header.Type == syscall.SCM_RIGHTS:
			rights, e := syscall.ParseUnixRights(&message)
			if e != nil {
				valid = false
				continue
			}
			for _, fd := range rights {
				files = append(files, os.NewFile(uintptr(fd), "native-broker-received"))
			}
		case message.Header.Level == syscall.SOL_SOCKET && message.Header.Type == syscall.SCM_CREDENTIALS:
			credentials++
			cred, e := syscall.ParseUnixCredentials(&message)
			if e != nil || uint32(cred.Pid) != b.ServerPID || cred.Uid != 0 {
				valid = false
			}
		default:
			valid = false
		}
	}
	if err != nil || parseErr != nil || !valid || credentials != 1 || flags&(syscall.MSG_TRUNC|syscall.MSG_CTRUNC) != 0 || n == 0 || b.CheckServer(pidfd) != nil {
		return fail(files)
	}
	frame, err := DecodeFrame(data[:n], len(files))
	if err != nil || frame.CheckBinding(b.Binding, frame.Header.RequestID) != nil {
		return fail(files)
	}
	return frame, files, nil
}
