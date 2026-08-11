package metrics

import (
	"bytes"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/common/expfmt"
)

func TestPloyzMetricsOracle(t *testing.T) {
	if got := Status(nil); got != Ok {
		t.Fatalf("Status(nil) = %q, want %q", got, Ok)
	}
	if got := Status(errors.New("failed")); got != Err {
		t.Fatalf("Status(error) = %q, want %q", got, Err)
	}

	Version.WithLabelValues("v1.2.3").Set(1)
	DNSQuery.WithLabelValues("false", Ok).Inc()

	families, err := prometheus.DefaultGatherer.Gather()
	if err != nil {
		t.Fatal(err)
	}

	var output bytes.Buffer
	for _, family := range families {
		if !strings.HasPrefix(family.GetName(), "uncloud_") {
			continue
		}
		if _, err := expfmt.MetricFamilyToText(&output, family); err != nil {
			t.Fatal(err)
		}
	}

	fmt.Printf("PLOYZ_ORACLE_BEGIN\n%sPLOYZ_ORACLE_END\n", output.String())
}
