package version

import (
	"fmt"
	"testing"
)

func TestPloyzVersionOracle(t *testing.T) {
	info := Info{
		Version:   "v1.2.3",
		GitCommit: "0123456789abcdef",
		GitState:  "dirty",
		BuildDate: "2026-08-11T01:02:03",
		BuiltBy:   "goreleaser",
		GoVersion: "go1.26.1",
		Platform:  "linux/amd64",
	}

	json, err := info.JSONString()
	if err != nil {
		t.Fatal(err)
	}
	fmt.Printf("PLOYZ_ORACLE_TEXT=%x\n", []byte(info.String()))
	fmt.Printf("PLOYZ_ORACLE_JSON=%x\n", []byte(json))

	controls := info
	controls.Version = "v\tX"
	controls.GitCommit = "commit\ncontinued\tcolumn"
	controls.BuiltBy = "builder\vsoft\fform"
	fmt.Printf("PLOYZ_ORACLE_CONTROL_TEXT=%x\n", []byte(controls.String()))

	for _, injected := range []string{"true", "false", "", "invalid", "TRUE"} {
		commit, dirty, date, builtBy = "", injected, "", ""
		actual := GetInfo()
		fmt.Printf("PLOYZ_ORACLE_DIRTY_%x=%x\n", []byte(injected), []byte(actual.GitState))
	}

	version = ""
	fmt.Printf("PLOYZ_ORACLE_DEVEL=%x\n", []byte(String()))
	version = "v9.8.7"
	fmt.Printf("PLOYZ_ORACLE_RELEASE=%x\n", []byte(String()))
}
