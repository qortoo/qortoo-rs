// Command profile demonstrates CPU profile export to Pyroscope.
//
// Run from go/qortoo:
//
//	go run ./examples/observability/profile
package main

import (
	"fmt"
	"log"
	"os"
	"strconv"
	"time"

	"github.com/grafana/pyroscope-go"
	"github.com/qortoo/qortoo-rs/go/qortoo"
)

const (
	defaultPyroscopeURL    = "http://localhost:4040"
	defaultApplicationName = "qortoo-go-example-profile"
	defaultProfileSeconds  = 30
)

// cpuSink keeps the synthetic workload observable to the compiler.
var cpuSink uint64

func main() {
	if err := run(); err != nil {
		log.Fatal(err)
	}
}

func run() error {
	pyroscopeURL := envOrDefault("PYROSCOPE_URL", defaultPyroscopeURL)
	applicationName := envOrDefault("PYROSCOPE_APPLICATION_NAME", defaultApplicationName)
	profileSeconds, err := profileDurationSeconds()
	if err != nil {
		return err
	}

	fmt.Printf("Exporting profiles -> %s\n", pyroscopeURL)
	fmt.Printf("Application        -> %s\n", applicationName)
	fmt.Println("View in Grafana    -> http://localhost:3000 -> Explore -> Pyroscope")
	fmt.Printf("Running workload for %ds...\n", profileSeconds)

	profiler, err := pyroscope.Start(pyroscope.Config{
		ApplicationName: applicationName,
		ServerAddress:   pyroscopeURL,
		Logger:          pyroscope.StandardLogger,
		Tags: map[string]string{
			"example": "profile",
			"library": "qortoo",
		},
		ProfileTypes: []pyroscope.ProfileType{
			pyroscope.ProfileCPU,
			pyroscope.ProfileAllocObjects,
			pyroscope.ProfileInuseObjects,
		},
	})
	if err != nil {
		return fmt.Errorf("start Pyroscope profiler: %w", err)
	}

	workloadErr := runCounterWorkload(time.Duration(profileSeconds) * time.Second)
	stopErr := profiler.Stop()
	if workloadErr != nil {
		return workloadErr
	}
	if stopErr != nil {
		return fmt.Errorf("stop Pyroscope profiler: %w", stopErr)
	}

	fmt.Println("Profile export finished.")
	return nil
}

func runCounterWorkload(duration time.Duration) error {
	connectivity := qortoo.NewLocalConnectivity()
	defer connectivity.Close()
	connectivity.SetRealtime(false)

	clientA, err := qortoo.NewClient(
		"example-profile",
		"client-a",
		qortoo.WithLocalConnectivity(connectivity),
	)
	if err != nil {
		return fmt.Errorf("create client-a: %w", err)
	}
	defer clientA.Close()

	clientB, err := qortoo.NewClient(
		"example-profile",
		"client-b",
		qortoo.WithLocalConnectivity(connectivity),
	)
	if err != nil {
		return fmt.Errorf("create client-b: %w", err)
	}
	defer clientB.Close()

	counterA, err := clientA.CreateCounter("profile-counter", nil)
	if err != nil {
		return fmt.Errorf("create counter: %w", err)
	}
	defer counterA.Close()

	counterB, err := clientB.SubscribeCounter("profile-counter", nil)
	if err != nil {
		return fmt.Errorf("subscribe counter: %w", err)
	}
	defer counterB.Close()

	startedAt := time.Now()
	var iteration uint64
	for time.Since(startedAt) < duration {
		iteration++
		if _, err := counterA.IncreaseBy(int64(iteration%13 + 1)); err != nil {
			return fmt.Errorf("increase counter: %w", err)
		}
		cpuSink = burnCPU(iteration)
		if err := counterA.Sync(); err != nil {
			return fmt.Errorf("sync client-a: %w", err)
		}
		if err := counterB.Sync(); err != nil {
			return fmt.Errorf("sync client-b: %w", err)
		}

		if iteration%10_000 == 0 {
			fmt.Printf(
				"iteration %8d: client-a=%d, client-b=%d\n",
				iteration,
				counterA.Value(),
				counterB.Value(),
			)
		}
	}

	fmt.Printf(
		"final counter values: client-a=%d, client-b=%d\n",
		counterA.Value(),
		counterB.Value(),
	)
	return nil
}

func burnCPU(seed uint64) uint64 {
	value := seed * 0x9E3779B97F4A7C15
	for round := uint64(0); round < 2_048; round++ {
		value = value<<7 | value>>(64-7)
		value ^= round
		value *= 0xBF58476D1CE4E5B9
	}
	return value
}

func envOrDefault(name, fallback string) string {
	if value := os.Getenv(name); value != "" {
		return value
	}
	return fallback
}

func profileDurationSeconds() (int, error) {
	raw := os.Getenv("QORTOO_PROFILE_SECONDS")
	if raw == "" {
		return defaultProfileSeconds, nil
	}
	seconds, err := strconv.Atoi(raw)
	if err != nil || seconds <= 0 {
		return 0, fmt.Errorf("QORTOO_PROFILE_SECONDS must be a positive integer, got %q", raw)
	}
	return seconds, nil
}
