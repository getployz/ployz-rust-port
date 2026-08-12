package store

import (
	"encoding/json"
	"testing"

	"github.com/psviderski/uncloud/internal/machine/api/pb"
	"github.com/stretchr/testify/require"
	"google.golang.org/protobuf/encoding/protojson"
)

func TestMachineJSONFixture(t *testing.T) {
	machine := &pb.MachineInfo{
		Id:   "m1",
		Name: "alpha",
		Network: &pb.NetworkConfig{
			PublicKey: []byte{
				7, 7, 7, 7, 7, 7, 7, 7,
				7, 7, 7, 7, 7, 7, 7, 7,
				7, 7, 7, 7, 7, 7, 7, 7,
				7, 7, 7, 7, 7, 7, 7, 7,
			},
		},
		PublicIp:      &pb.IP{Ip: []byte{192, 0, 2, 7}},
		DaemonVersion: "1.2.3",
		DockerVersion: "28.0",
		Hostname:      "node.example",
		Arch:          "amd64",
		OsPrettyName:  "Ployz OS",
		KernelVersion: "6.8",
	}

	encoded, err := protojson.Marshal(machine)
	require.NoError(t, err)
	require.Equal(t,
		`{"id":"m1","name":"alpha","network":{"publicKey":"BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc="},"publicIp":{"ip":"wAACBw=="},"daemonVersion":"1.2.3","dockerVersion":"28.0","hostname":"node.example","arch":"amd64","osPrettyName":"Ployz OS","kernelVersion":"6.8"}`,
		string(encoded))

	special, err := protojson.Marshal(&pb.MachineInfo{Name: "<>&\u2028\u2029"})
	require.NoError(t, err)
	require.Equal(t, "{\"name\":\"<>&\u2028\u2029\"}", string(special))

	var decoded pb.MachineInfo
	err = (protojson.UnmarshalOptions{DiscardUnknown: true}).Unmarshal([]byte(
		`{"id":null,"network":{"publicKey":"/x==","endpoints":null,"future":true},"public_ip":{"ip":"_w"},"unknown":"ignored"}`), &decoded)
	require.NoError(t, err)
	require.Empty(t, decoded.Id)
	require.Equal(t, []byte{0xff}, decoded.Network.PublicKey)
	require.Equal(t, []byte{0xff}, decoded.PublicIp.Ip)

	for _, duplicate := range []string{
		`{"id":"first","id":"second"}`,
		`{"daemonVersion":"one","daemon_version":"two"}`,
		`{"network":{"publicKey":"Bw==","public_key":"CA=="}}`,
		`{"network":{"endpoints":[{"port":1,"port":"2"}]}}`,
	} {
		err = (protojson.UnmarshalOptions{DiscardUnknown: true}).Unmarshal([]byte(duplicate), &pb.MachineInfo{})
		require.Error(t, err, duplicate)
	}

	blob, err := json.Marshal([]byte{0, 255})
	require.NoError(t, err)
	require.Equal(t, `"AP8="`, string(blob))
}
