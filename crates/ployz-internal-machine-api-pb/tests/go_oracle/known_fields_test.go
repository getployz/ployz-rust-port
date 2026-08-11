package pb

import (
	"encoding/hex"
	"fmt"
	"net/netip"
	"os"
	"strings"
	"testing"
	"time"

	statuspb "google.golang.org/genproto/googleapis/rpc/status"
	"google.golang.org/grpc/codes"
	grpcstatus "google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/anypb"
	"google.golang.org/protobuf/types/known/durationpb"
	"google.golang.org/protobuf/types/known/timestamppb"
)

type knownFieldFixture struct {
	name    string
	message proto.Message
}

func knownFieldFixtures() []knownFieldFixture {
	return []knownFieldFixture{
		{
			name:    "common/ip-v4",
			message: &IP{Ip: []byte{192, 0, 2, 1}},
		},
		{
			name: "common/ip-port-v6",
			message: &IPPort{
				Ip:   &IP{Ip: []byte{0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1}},
				Port: 51820,
			},
		},
		{
			name: "common/status-any",
			message: &Metadata{
				MachineId:   "machine-a",
				MachineName: "alpha",
				MachineAddr: "10.0.0.2:51000",
				Error:       "upstream failed",
				Status: &statuspb.Status{
					Code:    5,
					Message: "missing",
					Details: []*anypb.Any{{TypeUrl: "type.googleapis.com/example.Detail", Value: []byte{0x08, 0x2a}}},
				},
			},
		},
		{
			name: "common/log-entry-enum-timestamp",
			message: &LogEntry{
				Stream:    LogEntry_StreamType(777),
				Timestamp: timestamppb.New(time.Unix(1_700_000_000, 123_456_789).UTC()),
				Message:   []byte("line\n"),
			},
		},
		{
			name: "machine/update-optional-presence",
			message: &UpdateMachineRequest{
				Name:     proto.String(""),
				PublicIp: &IP{},
				Endpoints: []*IPPort{{
					Ip:   &IP{Ip: []byte{10, 0, 0, 10}},
					Port: 65537,
				}},
			},
		},
		{
			name: "machine/init-oneof-false",
			message: &InitClusterRequest{
				MachineName:    "alpha",
				Network:        &IPPrefix{Ip: &IP{Ip: []byte{10, 210, 0, 0}}, Bits: 24},
				PublicIpConfig: &InitClusterRequest_PublicIpAuto{PublicIpAuto: false},
				WireguardPort:  51820,
				WireguardMtu:   1420,
			},
		},
		{
			name: "machine/join-map-negative",
			message: &JoinClusterRequest{
				Machine: &MachineInfo{Id: "machine-a", Name: "alpha"},
				MinStoreVersion: map[string]int64{
					"actor-b": 42,
					"actor-a": -1,
				},
			},
		},
		{
			name: "machine/details-maps-duration",
			message: &MachineDetails{
				Metadata: &Metadata{MachineId: "machine-a"},
				Machine:  &MachineInfo{Id: "machine-a"},
				Rtts: map[string]*RTTStats{
					"machine-b": {
						Median: durationpb.New(12*time.Millisecond + 345*time.Microsecond),
						StdDev: durationpb.New(2 * time.Millisecond),
					},
				},
				StoreVersion: map[string]int64{"actor-a": 9},
			},
		},
		{
			name: "cluster/dns-unknown-enum",
			message: &CreateDomainRecordsRequest{Records: []*DNSRecord{{
				Name:   "app.example.test",
				Type:   DNSRecord_RecordType(777),
				Values: []string{"192.0.2.1", "2001:db8::1"},
			}}},
		},
		{
			name: "caddy/config-timestamp",
			message: &GetCaddyConfigResponse{
				Caddyfile:  "example.test { respond ok }",
				ModifiedAt: timestamppb.New(time.Unix(1_700_000_001, 0).UTC()),
			},
		},
		{
			name: "docker/exec-config-oneof",
			message: &ExecContainerRequest{Payload: &ExecContainerRequest_Config{Config: &ExecConfig{
				ContainerId: "container-a",
				Options:     []byte{0x7b, 0x7d},
			}}},
		},
		{
			name:    "docker/exec-stdin-oneof",
			message: &ExecContainerRequest{Payload: &ExecContainerRequest_Stdin{Stdin: []byte{0, 1, 2}}},
		},
		{
			name: "docker/exec-resize-oneof",
			message: &ExecContainerRequest{Payload: &ExecContainerRequest_Resize{Resize: &ResizeEvent{
				Height: 24,
				Width:  80,
			}}},
		},
		{
			name:    "docker/exec-response-empty-exit-code",
			message: &ExecContainerResponse{Payload: &ExecContainerResponse_ExitCode{ExitCode: 0}},
		},
		{
			name: "docker/service-container-enum",
			message: &CreateServiceContainerRequest{
				ServiceId:     "service-a",
				ServiceSpec:   []byte{0x7b, 0x7d},
				ContainerName: "service-a-1",
				ContainerType: CreateServiceContainerRequest_PRE_DEPLOY,
			},
		},
	}
}

func TestAddressHelperContracts(t *testing.T) {
	if _, err := (&IP{}).ToAddr(); err == nil || err.Error() != "invalid IP" {
		t.Fatalf("empty IP error: %v", err)
	}
	if _, err := (&IP{Ip: []byte{1, 2, 3}}).ToAddr(); err == nil || err.Error() != "unmarshal IP: unexpected slice size" {
		t.Fatalf("short IP error: %v", err)
	}

	scopedBytes := append(netip.MustParseAddr("2001:db8::7").AsSlice(), []byte("en0")...)
	scoped, err := (&IP{Ip: scopedBytes}).ToAddr()
	if err != nil {
		t.Fatal(err)
	}
	if scoped.Zone() != "en0" {
		t.Fatalf("zone: %q", scoped.Zone())
	}
	if got := NewIP(scoped).Ip; !proto.Equal(&IP{Ip: got}, &IP{Ip: scopedBytes}) {
		t.Fatalf("scoped round trip: %x", got)
	}

	addrPort, err := (&IPPort{Ip: NewIP(netip.MustParseAddr("192.0.2.7")), Port: 65537}).ToAddrPort()
	if err != nil {
		t.Fatal(err)
	}
	if addrPort.Port() != 1 {
		t.Fatalf("narrowed port: %d", addrPort.Port())
	}

	err = (&NetworkConfig{ManagementIp: &IP{Ip: []byte{1, 2, 3}}, PublicKey: make([]byte, KeyLen)}).Validate()
	if grpcstatus.Code(err) != codes.InvalidArgument || grpcstatus.Convert(err).Message() != "invalid management IP: unmarshal IP: unexpected slice size" {
		t.Fatalf("management IP validation: %v", err)
	}

	func() {
		defer func() {
			if recover() == nil {
				t.Fatal("missing endpoint IP did not panic")
			}
		}()
		_ = (&NetworkConfig{Endpoints: []*IPPort{{}}, PublicKey: make([]byte, KeyLen)}).Validate()
	}()
}

func TestKnownFieldFixtures(t *testing.T) {
	marshal := proto.MarshalOptions{Deterministic: true}
	rustFixtures := readFixtureExchange(t, os.Getenv("PLOYZ_RUST_FIXTURES_IN"))
	seen := make(map[string]bool, len(knownFieldFixtures()))
	var goFixtures strings.Builder
	for _, fixture := range knownFieldFixtures() {
		t.Run(fixture.name, func(t *testing.T) {
			if seen[fixture.name] {
				t.Fatalf("duplicate fixture %q", fixture.name)
			}
			seen[fixture.name] = true

			encoded, err := marshal.Marshal(fixture.message)
			if err != nil {
				t.Fatal(err)
			}
			fmt.Fprintf(&goFixtures, "%s\t%s\n", fixture.name, hex.EncodeToString(encoded))

			if len(rustFixtures) > 0 {
				rustWire, ok := rustFixtures[fixture.name]
				if !ok {
					t.Fatalf("Rust did not emit fixture %q", fixture.name)
				}
				fromRust := fixture.message.ProtoReflect().Type().New().Interface()
				if err := proto.Unmarshal(rustWire, fromRust); err != nil {
					t.Fatalf("decode Rust fixture: %v", err)
				}
				if !proto.Equal(fixture.message, fromRust) {
					t.Fatalf("Rust fixture differs\nwant: %v\n got: %v", fixture.message, fromRust)
				}
			}

			decoded := fixture.message.ProtoReflect().Type().New().Interface()
			if err := proto.Unmarshal(encoded, decoded); err != nil {
				t.Fatal(err)
			}
			if !proto.Equal(fixture.message, decoded) {
				t.Fatalf("round trip differs\nwant: %v\n got: %v", fixture.message, decoded)
			}
		})
	}
	if len(rustFixtures) > 0 && len(rustFixtures) != len(seen) {
		t.Fatalf("Rust emitted %d fixtures; Go owns %d", len(rustFixtures), len(seen))
	}
	if path := os.Getenv("PLOYZ_GO_FIXTURES_OUT"); path != "" {
		if err := os.WriteFile(path, []byte(goFixtures.String()), 0o600); err != nil {
			t.Fatal(err)
		}
	}
}

func readFixtureExchange(t *testing.T, path string) map[string][]byte {
	t.Helper()
	if path == "" {
		return nil
	}
	contents, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	fixtures := make(map[string][]byte)
	for _, line := range strings.Split(strings.TrimSpace(string(contents)), "\n") {
		name, wireHex, ok := strings.Cut(line, "\t")
		if !ok || name == "" || wireHex == "" {
			t.Fatalf("invalid fixture row %q", line)
		}
		if _, duplicate := fixtures[name]; duplicate {
			t.Fatalf("duplicate fixture %q", name)
		}
		wire, err := hex.DecodeString(wireHex)
		if err != nil {
			t.Fatalf("fixture %q: %v", name, err)
		}
		fixtures[name] = wire
	}
	return fixtures
}
