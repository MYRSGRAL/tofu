package nativehooks

import (
	"encoding/binary"
	"errors"
	"net/netip"
	"strings"
	"time"
	"unicode/utf8"
)

// FrozenSpec — единственный источник wire bytes; fixture136 не участвует.
const FrozenSpec = "260bc50eeb4f3a40ac97cd9a54ff0ea943a833ef"
const (
	HeaderBytes              = 64
	MaxFrameBytes            = 768
	MaxPathBytes             = 512
	OwnerTTL                 = 20 * time.Millisecond
	BootstrapTTL             = time.Second
	OwnedTUN          uint64 = 1
	OwnerLookup       uint64 = 2
	StrictAppRequired uint64 = 4
	AllFeatures              = OwnedTUN | OwnerLookup | StrictAppRequired
)

var ErrMalformed = errors.New("некорректный YNKBROK1 frame")

type Kind uint16

const (
	HelloKind     Kind = 1
	FeaturesKind  Kind = 2
	AttachKind    Kind = 3
	AttachACKKind Kind = 4
	LookupKind    Kind = 16
	ReplyKind     Kind = 17
	RevokeKind    Kind = 32
)

type Header struct {
	Kind      Kind
	RequestID uint64
	Epoch     uint64
	Digest    [32]byte
}
type HelloBody struct {
	Session     [16]byte
	Generation  uint64
	Invocation  [16]byte
	OwnerTTL    time.Duration
	Deadline    time.Duration
	Supported   uint64
	AppRequired bool
}
type AttachBody struct {
	Ifindex uint32
	Flags   uint32
	MTU     uint32
	Name    string
}

// ReplyBody.Cookie информационный: никогда не входит в EncodeLookup.
type ReplyBody struct {
	Status           Status
	Reason           uint8
	TTL              time.Duration
	Cookie           uint64
	PID              uint32
	Starttime        uint64
	ExecutableDevice uint64
	ExecutableInode  uint64
	Path             string
}
type Frame struct {
	Header       Header
	Nonce        [32]byte
	Hello        HelloBody
	Supported    uint64
	Selected     uint64
	Attach       AttachBody
	Lookup       Flow
	Reply        ReplyBody
	RevokeReason uint32
}

var be = binary.BigEndian

func zero(b []byte) bool {
	for _, v := range b {
		if v != 0 {
			return false
		}
	}
	return true
}
func u16(b []byte, n int) uint16                       { return be.Uint16(b[n : n+2]) }
func u32(b []byte, n int) uint32                       { return be.Uint32(b[n : n+4]) }
func u64(b []byte, n int) uint64                       { return be.Uint64(b[n : n+8]) }
func boundedDuration(n uint64, max time.Duration) bool { return n > 0 && n <= uint64(max) }
func validPath(b []byte) bool {
	return len(b) > 0 && len(b) <= MaxPathBytes && b[0] == '/' && !strings.ContainsRune(string(b), 0) &&
		!strings.HasSuffix(string(b), " (deleted)") && utf8.Valid(b)
}
func interfaceName(b []byte) (string, error) {
	n := 0
	for n < len(b) && b[n] != 0 {
		v := b[n]
		if !(v >= 'a' && v <= 'z' || v >= 'A' && v <= 'Z' || v >= '0' && v <= '9' || v == '_' || v == '.' || v == '-') {
			return "", ErrMalformed
		}
		n++
	}
	if n == 0 || n == len(b) || !zero(b[n:]) {
		return "", ErrMalformed
	}
	return string(b[:n]), nil
}

// DecodeFrame проверяет bytes/count, но НЕ credentials, pidfd или unit authority.
// Полученные FD принадлежат transport и закрываются им при любой ошибке.
func DecodeFrame(b []byte, fdCount int) (Frame, error) {
	fail := func() (Frame, error) { return Frame{}, ErrMalformed }
	if len(b) < HeaderBytes || len(b) > MaxFrameBytes || string(b[:8]) != "YNKBROK1" || u16(b, 8) != 1 || uint64(u32(b, 12)) != uint64(len(b)) {
		return fail()
	}
	kind := Kind(u16(b, 10))
	size := 0
	fds := 0
	switch kind {
	case HelloKind:
		size = 176
	case FeaturesKind:
		size = 112
	case AttachKind:
		size = 128
		fds = 1
	case AttachACKKind, RevokeKind:
		size = 104
	case LookupKind:
		size = 144
	case ReplyKind:
		if len(b) < 160 || len(b) > 160+MaxPathBytes {
			return fail()
		}
	default:
		return fail()
	}
	if fdCount != fds || size != 0 && len(b) != size {
		return fail()
	}
	id, epoch := u64(b, 16), u64(b, 24)
	if ((kind == LookupKind || kind == ReplyKind) != (id != 0)) || epoch == 0 || zero(b[32:64]) || zero(b[64:96]) {
		return fail()
	}
	f := Frame{Header: Header{Kind: kind, RequestID: id, Epoch: epoch}}
	copy(f.Header.Digest[:], b[32:64])
	copy(f.Nonce[:], b[64:96])
	switch kind {
	case HelloKind:
		ttl, deadline, supported, flags := u64(b, 144), u64(b, 152), u64(b, 160), u64(b, 168)
		if zero(b[96:112]) || u64(b, 112) == 0 || zero(b[120:136]) || u32(b, 136) != MaxFrameBytes || u16(b, 140) != 1 || !zero(b[142:144]) ||
			!boundedDuration(ttl, OwnerTTL) || !boundedDuration(deadline, BootstrapTTL) || supported & ^AllFeatures != 0 || supported&OwnedTUN == 0 || flags & ^uint64(1) != 0 || flags != 0 && supported&AllFeatures != AllFeatures {
			return fail()
		}
		copy(f.Hello.Session[:], b[96:112])
		copy(f.Hello.Invocation[:], b[120:136])
		f.Hello.Generation = u64(b, 112)
		f.Hello.OwnerTTL = time.Duration(ttl)
		f.Hello.Deadline = time.Duration(deadline)
		f.Hello.Supported = supported
		f.Hello.AppRequired = flags != 0
	case FeaturesKind:
		f.Supported = u64(b, 96)
		f.Selected = u64(b, 104)
		if f.Supported & ^AllFeatures != 0 || f.Selected & ^f.Supported != 0 || f.Selected&OwnedTUN == 0 {
			return fail()
		}
	case AttachKind:
		name, err := interfaceName(b[112:128])
		mtu := u32(b, 108)
		if err != nil || u32(b, 96) != 1 || u32(b, 100) == 0 || u32(b, 104) != 0x1001 || mtu < 68 || mtu > 65535 {
			return fail()
		}
		f.Attach = AttachBody{Ifindex: u32(b, 100), Flags: u32(b, 104), MTU: mtu, Name: name}
	case AttachACKKind:
		if u32(b, 96) != 1 || !zero(b[100:104]) {
			return fail()
		}
	case LookupKind:
		if (b[96] != 4 && b[96] != 6) || (b[97] != 6 && b[97] != 17) || !zero(b[98:100]) || !zero(b[136:144]) || u16(b, 100) == 0 || u16(b, 102) == 0 {
			return fail()
		}
		var local, remote netip.Addr
		if b[96] == 4 {
			if !zero(b[108:120]) || !zero(b[124:136]) {
				return fail()
			}
			local = netip.AddrFrom4([4]byte(b[104:108]))
			remote = netip.AddrFrom4([4]byte(b[120:124]))
		} else {
			local = netip.AddrFrom16([16]byte(b[104:120]))
			remote = netip.AddrFrom16([16]byte(b[120:136]))
		}
		f.Lookup = Flow{Protocol: b[97], Local: netip.AddrPortFrom(local, u16(b, 100)), Remote: netip.AddrPortFrom(remote, u16(b, 102))}
		if !f.Lookup.valid() {
			return fail()
		}
	case ReplyKind:
		n := int(u16(b, 100))
		status, reason := b[96], b[97]
		if len(b) != 160+n || n > MaxPathBytes || !zero(b[98:100]) || !zero(b[102:104]) || !zero(b[124:128]) || !zero(b[152:160]) {
			return fail()
		}
		switch {
		case status == 1 && reason == 0:
			if !boundedDuration(u64(b, 104), OwnerTTL) || u32(b, 120) == 0 || u64(b, 128) == 0 || !validPath(b[160:]) {
				return fail()
			}
			f.Reply = ReplyBody{Status: Known, TTL: time.Duration(u64(b, 104)), Cookie: u64(b, 112), PID: u32(b, 120), Starttime: u64(b, 128), ExecutableDevice: u64(b, 136), ExecutableInode: u64(b, 144), Path: string(b[160:])}
		case status == 2 && reason == 0, status == 3 && reason >= 1 && reason <= 7:
			if n != 0 || !zero(b[104:152]) {
				return fail()
			}
			f.Reply.Status = Unknown
			if status == 3 {
				f.Reply.Status = Unavailable
			}
			f.Reply.Reason = reason
		default:
			return fail()
		}
	case RevokeKind:
		f.RevokeReason = u32(b, 96)
		if f.RevokeReason < 1 || f.RevokeReason > 7 || !zero(b[100:104]) {
			return fail()
		}
	}
	return f, nil
}

func (f Frame) CheckBinding(expected Binding, requestID uint64) error {
	if f.Header.Epoch != expected.Epoch || f.Header.Digest != expected.ConfigDigest || f.Nonce != expected.Nonce || f.Header.RequestID != requestID {
		return ErrMalformed
	}
	return nil
}
func (f Frame) SelectedFeatures(appRequired bool, offered uint64) (uint64, error) {
	required := OwnedTUN
	if appRequired {
		required = AllFeatures
	}
	if f.Header.Kind != FeaturesKind || offered & ^AllFeatures != 0 || f.Selected & ^offered != 0 || f.Selected&required != required {
		return 0, ErrMalformed
	}
	return f.Selected, nil
}
func makeFrame(kind Kind, id uint64, binding Binding, size int) []byte {
	b := make([]byte, size)
	copy(b, "YNKBROK1")
	be.PutUint16(b[8:], 1)
	be.PutUint16(b[10:], uint16(kind))
	be.PutUint32(b[12:], uint32(size))
	be.PutUint64(b[16:], id)
	be.PutUint64(b[24:], binding.Epoch)
	copy(b[32:64], binding.ConfigDigest[:])
	copy(b[64:96], binding.Nonce[:])
	return b
}

// EncodeLookup намеренно не принимает cookie/flags, PID/UID/path.
func EncodeLookup(binding Binding, id uint64, tuple Flow) ([]byte, error) {
	if !tuple.valid() {
		return nil, ErrMalformed
	}
	b := makeFrame(LookupKind, id, binding, 144)
	b[96] = 6
	if tuple.Local.Addr().Is4() {
		b[96] = 4
	}
	b[97] = tuple.Protocol
	be.PutUint16(b[100:], tuple.Local.Port())
	be.PutUint16(b[102:], tuple.Remote.Port())
	copy(b[104:120], tuple.Local.Addr().AsSlice())
	copy(b[120:136], tuple.Remote.Addr().AsSlice())
	if _, err := DecodeFrame(b, 0); err != nil {
		return nil, err
	}
	return b, nil
}
func EncodeFeatures(binding Binding, selected uint64, appRequired bool, offered uint64) ([]byte, error) {
	b := makeFrame(FeaturesKind, 0, binding, 112)
	be.PutUint64(b[96:], AllFeatures)
	be.PutUint64(b[104:], selected)
	f, err := DecodeFrame(b, 0)
	if err != nil {
		return nil, err
	}
	if _, err = f.SelectedFeatures(appRequired, offered); err != nil {
		return nil, err
	}
	return b, nil
}
func EncodeAttachACK(binding Binding) ([]byte, error) {
	b := makeFrame(AttachACKKind, 0, binding, 104)
	be.PutUint32(b[96:], 1)
	if _, err := DecodeFrame(b, 0); err != nil {
		return nil, err
	}
	return b, nil
}

// EffectiveReplyTTL вычитает полный monotonic RTT; ноль означает Unavailable.
func EffectiveReplyTTL(ttl time.Duration, sent, received time.Time) time.Duration {
	if ttl <= 0 || ttl > OwnerTTL || received.Before(sent) {
		return 0
	}
	rtt := received.Sub(sent)
	if rtt >= ttl {
		return 0
	}
	return ttl - rtt
}

// Sequence вызывается только единственным transport owner, без параллельных обменов.
type Sequence struct {
	last    uint64
	pending uint64
	retired bool
}

func (s *Sequence) Begin(id uint64) error {
	if s.retired || s.pending != 0 || id == 0 || id <= s.last {
		s.Retire()
		return ErrMalformed
	}
	s.last = id
	s.pending = id
	return nil
}
func (s *Sequence) Complete(id uint64) error {
	if s.retired || s.pending == 0 || s.pending != id {
		s.Retire()
		return ErrMalformed
	}
	s.pending = 0
	return nil
}
func (s *Sequence) Retire() { s.pending = 0; s.retired = true }
