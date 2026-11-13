package nativehooks

import (
	"io"
	"net"
	"net/netip"
	"testing"
	"time"
)

func fixture(t *testing.T) (*Session, Evidence, time.Time) {
	t.Helper()
	now := time.Now()
	binding := Binding{Nonce: [32]byte{1}, ConfigDigest: [32]byte{2}, Epoch: 3}
	session, err := NewSession(binding, 20*time.Millisecond)
	if err != nil {
		t.Fatal(err)
	}
	return session, Evidence{Flow: Flow{Protocol: 6, Local: netip.MustParseAddrPort("127.0.0.1:1234"), Remote: netip.MustParseAddrPort("127.0.0.2:443")}, Binding: binding, Status: Known, Observed: now, Expires: now.Add(20 * time.Millisecond), ExecutableInode: 4}, now
}

func TestProofMatrix(t *testing.T) {
	cases := []struct {
		name    string
		change  func(*Evidence)
		elapsed time.Duration
		deny    bool
	}{
		{name: "known"},
		{name: "other-flow", change: func(p *Evidence) { p.Flow.Remote = netip.MustParseAddrPort("127.0.0.3:443") }, deny: true},
		{name: "expiry", elapsed: 20 * time.Millisecond, deny: true},
		{name: "unknown", change: func(p *Evidence) { p.Status = Unknown }, deny: true},
		{name: "unavailable", change: func(p *Evidence) { p.Status = Unavailable }, deny: true},
		{name: "nonce", change: func(p *Evidence) { p.Binding.Nonce[0]++ }, deny: true},
		{name: "digest", change: func(p *Evidence) { p.Binding.ConfigDigest[0]++ }, deny: true},
		{name: "epoch", change: func(p *Evidence) { p.Binding.Epoch++ }, deny: true},
		{name: "caller-path", change: func(p *Evidence) { p.RawCallerPath = "/usr/bin/trusted" }, deny: true},
		{name: "long-ttl", change: func(p *Evidence) { p.Expires = p.Expires.Add(time.Millisecond) }, deny: true},
		{name: "future", change: func(p *Evidence) { p.Observed = p.Observed.Add(time.Millisecond) }, deny: true},
		{name: "missing-identity", change: func(p *Evidence) { p.ExecutableInode = 0 }, deny: true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			s, p, now := fixture(t)
			tuple := p.Flow
			if c.change != nil {
				c.change(&p)
			}
			for _, stage := range []Stage{BeforeRouting, AfterDNS, BeforeDial} {
				if err := s.Guard(now.Add(c.elapsed), stage, true, tuple, p); (err != nil) != c.deny {
					t.Fatalf("stage=%d err=%v", stage, err)
				}
			}
		})
	}
}

func TestDNSWaitRequiresFreshProof(t *testing.T) {
	s, p, now := fixture(t)
	if err := s.Guard(now, BeforeRouting, true, p.Flow, p); err != nil {
		t.Fatal(err)
	}
	for _, stage := range []Stage{AfterDNS, BeforeDial} {
		if s.Guard(now.Add(21*time.Millisecond), stage, true, p.Flow, p) == nil {
			t.Fatal("просроченный proof допущен")
		}
	}
	if err := s.Guard(now, BeforeRouting, false, Flow{}, Evidence{}); err != nil {
		t.Fatal("non-app control", err)
	}
}

func TestRevokeClosesTCPUDPFlows(t *testing.T) {
	for _, reason := range []string{"revoke", "EOF", "cancel", "broker-death"} {
		t.Run(reason, func(t *testing.T) {
			s, p, now := fixture(t)
			tcp, peer := net.Pipe()
			defer peer.Close()
			udp, err := net.ListenPacket("udp4", "127.0.0.1:0")
			if err != nil {
				t.Fatal(err)
			}
			for _, flow := range []io.Closer{tcp, udp} {
				if _, err := s.Track(now, true, p.Flow, p, flow); err != nil {
					t.Fatal(err)
				}
			}
			if err := s.Revoke(); err != nil {
				t.Fatal(err)
			}
			if _, err := tcp.Write([]byte{1}); err == nil {
				t.Fatal("TCP остался открыт")
			}
			if _, err := udp.WriteTo([]byte{1}, udp.LocalAddr()); err == nil {
				t.Fatal("UDP остался открыт")
			}
			if s.Context().Err() == nil {
				t.Fatal("pending I/O не отменён")
			}
			if s.Guard(now, BeforeDial, true, p.Flow, p) == nil {
				t.Fatal("revoke не блокирует dial")
			}
			if err := s.Revoke(); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestCanonicalTCPUDPv4v6(t *testing.T) {
	for _, protocol := range []uint8{6, 17} {
		for _, pair := range [][2]string{{"192.0.2.1:1234", "192.0.2.2:443"}, {"[2001:db8::1]:1234", "[2001:db8::2]:443"}} {
			s, p, now := fixture(t)
			p.Flow = Flow{Protocol: protocol, Local: netip.MustParseAddrPort(pair[0]), Remote: netip.MustParseAddrPort(pair[1])}
			if err := s.Guard(now, BeforeDial, true, p.Flow, p); err != nil {
				t.Fatal(protocol, pair, err)
			}
		}
	}
	for _, pair := range [][2]string{{"[::ffff:192.0.2.1]:1234", "[::ffff:192.0.2.2]:443"}, {"192.0.2.1:0", "192.0.2.2:443"}, {"192.0.2.1:1234", "[2001:db8::2]:443"}} {
		s, p, now := fixture(t)
		p.Flow = Flow{Protocol: 6, Local: netip.MustParseAddrPort(pair[0]), Remote: netip.MustParseAddrPort(pair[1])}
		if s.Guard(now, BeforeDial, true, p.Flow, p) == nil {
			t.Fatal("неканонический tuple допущен", pair)
		}
	}
}

func TestTrackAfterRevokeDeniedAndReleaseIdempotent(t *testing.T) {
	s, p, now := fixture(t)
	tcp, peer := net.Pipe()
	defer peer.Close()
	handle, err := s.Track(now, true, p.Flow, p, tcp)
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Release(handle); err != nil {
		t.Fatal(err)
	}
	if err := s.Release(handle); err != nil {
		t.Fatal(err)
	}
	if err := s.Revoke(); err != nil {
		t.Fatal(err)
	}
	next, other := net.Pipe()
	defer next.Close()
	defer other.Close()
	if _, err := s.Track(now, true, p.Flow, p, next); err == nil {
		t.Fatal("flow опубликован после revoke")
	}
}
