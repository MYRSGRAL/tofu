// Package nativehooks задаёт внутреннюю семантику guard, без wire ABI.
package nativehooks

import (
	"context"
	"errors"
	"io"
	"net/netip"
	"sync"
	"time"
)

// MaxTrackedFlows ограничивает только этот fixture; production budget согласует Root.
const MaxTrackedFlows = 4096

var ErrDenied = errors.New("нет действующего broker proof")

type Status uint8

const (
	Unknown Status = iota
	Unavailable
	Known
)

type Stage uint8

const (
	BeforeRouting Stage = iota
	AfterDNS
	BeforeDial
)

// Binding поступает только из trusted bootstrap, не из native config/env.
type Binding struct {
	Nonce        [32]byte
	ConfigDigest [32]byte
	Epoch        uint64
}

// Flow хранит исходный canonical tuple до DNS/sniff переписывания назначения.
type Flow struct {
	Protocol uint8
	Local    netip.AddrPort
	Remote   netip.AddrPort
}

func (f Flow) valid() bool {
	a, b := f.Local.Addr(), f.Remote.Addr()
	return (f.Protocol == 6 || f.Protocol == 17) && f.Local.IsValid() && f.Remote.IsValid() &&
		f.Local.Port() != 0 && f.Remote.Port() != 0 && a.Zone() == "" && b.Zone() == "" &&
		!a.Is4In6() && !b.Is4In6() && a.Is4() == b.Is4() && !a.IsUnspecified() && !b.IsUnspecified()
}

// Evidence — semantic input trusted broker decoder; это не публичный assertion API.
// Строка caller path никогда не становится identity доказательством.
type Evidence struct {
	Flow             Flow
	Binding          Binding
	Status           Status
	Observed         time.Time
	Expires          time.Time
	ExecutableDevice uint64
	ExecutableInode  uint64
	RawCallerPath    string
}

// Handle передаёт сессии владение закрытием одного flow.
type Handle struct {
	flow io.Closer
	once sync.Once
	err  error
}

func (h *Handle) close() error {
	h.once.Do(func() { h.err = h.flow.Close() })
	return h.err
}

// Session отзывает flows и pending I/O; положительного cache здесь нет.
type Session struct {
	ctx        context.Context
	cancel     context.CancelFunc
	mu         sync.Mutex
	binding    Binding
	maxTTL     time.Duration
	revoked    bool
	revokeDone chan struct{}
	revokeErr  error
	flows      map[*Handle]struct{}
}

func NewSession(binding Binding, maxTTL time.Duration) (*Session, error) {
	if binding.Epoch == 0 || binding.Nonce == [32]byte{} || binding.ConfigDigest == [32]byte{} || maxTTL <= 0 {
		return nil, ErrDenied
	}
	ctx, cancel := context.WithCancel(context.Background())
	return &Session{ctx: ctx, cancel: cancel, binding: binding, maxTTL: maxTTL, revokeDone: make(chan struct{}), flows: make(map[*Handle]struct{})}, nil
}

func (s *Session) guard(now time.Time, stage Stage, required bool, tuple Flow, proof Evidence) error {
	if s.revoked || stage > BeforeDial || proof.RawCallerPath != "" {
		return ErrDenied
	}
	if !required {
		return nil
	}
	if !tuple.valid() || proof.Flow != tuple || proof.Status != Known || proof.Binding != s.binding || proof.ExecutableInode == 0 ||
		proof.Observed.IsZero() || now.Before(proof.Observed) || !now.Before(proof.Expires) ||
		proof.Expires.Sub(proof.Observed) <= 0 || proof.Expires.Sub(proof.Observed) > s.maxTTL {
		return ErrDenied
	}
	return nil
}

func (s *Session) Guard(now time.Time, stage Stage, required bool, tuple Flow, proof Evidence) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.guard(now, stage, required, tuple, proof)
}

// Track атомарно относительно revoke регистрирует flow до его публикации.
// Ошибка передаёт закрытие flow вызывающему коду; после успеха сессия владеет close.
func (s *Session) Track(now time.Time, required bool, tuple Flow, proof Evidence, flow io.Closer) (*Handle, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if flow == nil || len(s.flows) >= MaxTrackedFlows {
		return nil, ErrDenied
	}
	if err := s.guard(now, BeforeDial, required, tuple, proof); err != nil {
		return nil, err
	}
	handle := &Handle{flow: flow}
	s.flows[handle] = struct{}{}
	return handle, nil
}

// Release закрывает flow и снимает регистрацию, не удерживая mutex во время I/O.
func (s *Session) Release(flow *Handle) error {
	s.mu.Lock()
	_, exists := s.flows[flow]
	s.mu.Unlock()
	if !exists {
		return nil
	}
	err := flow.close()
	s.mu.Lock()
	delete(s.flows, flow)
	s.mu.Unlock()
	return err
}

// Revoke вызывается также на EOF/broker death/cancel; повтор безопасен.
func (s *Session) Revoke() error {
	s.mu.Lock()
	if s.revoked {
		done := s.revokeDone
		s.mu.Unlock()
		<-done
		return s.revokeErr
	}
	s.revoked = true
	s.cancel()
	flows := s.flows
	s.flows = make(map[*Handle]struct{})
	s.mu.Unlock()
	var result error
	for flow := range flows {
		result = errors.Join(result, flow.close())
	}
	s.revokeErr = result
	close(s.revokeDone)
	return result
}

// Context передаётся каждому DNS/dial ожиданию, чтобы revoke отменял и pending I/O.
func (s *Session) Context() context.Context { return s.ctx }
