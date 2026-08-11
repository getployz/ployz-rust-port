package main

import (
	"fmt"
	"os"
	"os/user"
)

func required(operation string, value *user.User, err error) *user.User {
	if err != nil {
		fmt.Fprintf(os.Stderr, "%s: %v\n", operation, err)
		os.Exit(1)
	}
	return value
}

func emit(operation string, value *user.User) {
	fmt.Printf(
		"%s\t%s\t%s\t%x\t%x\t%x\n",
		operation,
		value.Uid,
		value.Gid,
		[]byte(value.Username),
		[]byte(value.Name),
		[]byte(value.HomeDir),
	)
}

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: macos-nss-go-probe DIRECTORY_USER")
		os.Exit(2)
	}
	directoryName := os.Args[1]

	currentValue, err := user.Current()
	currentValue = required("Current", currentValue, err)

	currentByName, err := user.Lookup(currentValue.Username)
	currentByName = required("Lookup(current username)", currentByName, err)

	currentByID, err := user.LookupId(currentValue.Uid)
	currentByID = required("LookupId(current UID)", currentByID, err)

	directoryByName, err := user.Lookup(directoryName)
	directoryByName = required("Lookup(directory username)", directoryByName, err)

	directoryByID, err := user.LookupId(directoryByName.Uid)
	directoryByID = required("LookupId(directory UID)", directoryByID, err)

	emit("current", currentValue)
	emit("lookup_current_name", currentByName)
	emit("lookup_current_id", currentByID)
	emit("lookup_directory_name", directoryByName)
	emit("lookup_directory_id", directoryByID)
}
