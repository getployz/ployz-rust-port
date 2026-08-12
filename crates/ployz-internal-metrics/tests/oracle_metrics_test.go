package metrics

import (
	"bytes"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/prometheus/client_golang/prometheus"
	dto "github.com/prometheus/client_model/go"
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

	before := time.Now()
	DNSQuery.WithLabelValues("created", "probe")
	after := time.Now()
	first, err := prometheus.DefaultGatherer.Gather()
	if err != nil {
		t.Fatal(err)
	}
	time.Sleep(time.Millisecond)
	second, err := prometheus.DefaultGatherer.Gather()
	if err != nil {
		t.Fatal(err)
	}
	firstCreated := dnsCreatedTimestamp(t, first, "created", "probe")
	secondCreated := dnsCreatedTimestamp(t, second, "created", "probe")
	if firstCreated.Before(before) || firstCreated.After(after) {
		t.Fatalf("created timestamp %s is outside first access [%s, %s]", firstCreated, before, after)
	}
	if !firstCreated.Equal(secondCreated) {
		t.Fatalf("created timestamp changed across gathers: %s != %s", firstCreated, secondCreated)
	}
}

func dnsCreatedTimestamp(t *testing.T, families []*dto.MetricFamily, internal, status string) time.Time {
	t.Helper()
	for _, family := range families {
		if family.GetName() != "uncloud_dns_query_total" {
			continue
		}
		for _, metric := range family.GetMetric() {
			labels := make(map[string]string, len(metric.GetLabel()))
			for _, label := range metric.GetLabel() {
				labels[label.GetName()] = label.GetValue()
			}
			if labels["internal"] == internal && labels["status"] == status {
				if metric.GetCounter().GetCreatedTimestamp() == nil {
					t.Fatal("DNS counter has no created timestamp")
				}
				return metric.GetCounter().GetCreatedTimestamp().AsTime()
			}
		}
	}
	t.Fatalf("DNS counter child internal=%q status=%q was not gathered", internal, status)
	return time.Time{}
}
