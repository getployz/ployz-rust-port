package grpcversion

import (
	"context"
	"fmt"
	"strings"
	"testing"

	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

func TestPloyzGrpcVersionOracle(t *testing.T) {
	prefixes := []string{"", "v", "V", " "}
	cores := []string{
		"", "0", "00", "1", "01", "1.2", "01.002", "1.2.3", "1.2.3.4",
		"9223372036854775807", "9223372036854775808",
	}
	suffixes := []string{
		"", "-alpha", "-00", "-a..b", "+build", "+001", "-alpha+build", "-é", " ",
	}

	var output strings.Builder
	for _, prefix := range prefixes {
		for _, core := range cores {
			for _, suffix := range suffixes {
				input := prefix + core + suffix
				fmt.Fprintf(&output, "parse:%x=%s\n", []byte(input), parseVersionOrZero(input).String())
			}
		}
	}

	cases := []struct {
		name  string
		pairs []string
	}{
		{name: "missing"},
		{name: "malformed", pairs: []string{MetadataKeyClientVersion, "bad"}},
		{name: "client-old", pairs: []string{MetadataKeyClientVersion, "0.19.9"}},
		{name: "server-old", pairs: []string{
			MetadataKeyClientVersion, "999.0.0",
			MetadataKeyMinServerVersion, "999.0.0",
		}},
		{name: "accepted", pairs: []string{
			MetadataKeyClientVersion, "999.0.0",
			MetadataKeyMinServerVersion, "0.20.0",
		}},
		{name: "duplicate-first", pairs: []string{
			MetadataKeyClientVersion, "999.0.0",
			MetadataKeyClientVersion, "0.0.0",
			MetadataKeyMinServerVersion, "0.20.0",
		}},
	}
	for _, tc := range cases {
		ctx := context.Background()
		if tc.pairs != nil {
			ctx = metadata.NewIncomingContext(ctx, metadata.Pairs(tc.pairs...))
		}
		err := checkClientVersionHeaders(ctx)
		if err == nil {
			fmt.Fprintf(&output, "validate:%s=ok\n", tc.name)
			continue
		}
		grpcStatus := status.Convert(err)
		fmt.Fprintf(&output, "validate:%s=%d|%s\n", tc.name, grpcStatus.Code(), grpcStatus.Message())
	}

	fmt.Printf("PLOYZ_ORACLE_BEGIN\n%x\nPLOYZ_ORACLE_END\n", []byte(output.String()))
}
