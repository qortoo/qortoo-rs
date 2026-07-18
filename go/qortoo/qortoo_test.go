package qortoo

import (
	"fmt"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
)

func TestClientAndCounterLifecycle(t *testing.T) {
	client, err := NewClient("go-binding-test", "lifecycle")
	require.NoError(t, err)
	defer client.Close()

	require.Equal(t, "go-binding-test", client.Collection())
	require.Equal(t, "lifecycle", client.Alias())

	counter, err := client.CreateCounter("my-counter", nil)
	require.NoError(t, err)
	defer counter.Close()

	require.Equal(t, "my-counter", counter.Key())
	require.Equal(t, TypeCounter, counter.Type())
	require.Equal(t, StateCreating, counter.State())

	v, err := counter.Increase()
	require.NoError(t, err)
	require.Equal(t, int64(1), v)
	v, err = counter.IncreaseBy(4)
	require.NoError(t, err)
	require.Equal(t, int64(5), v)
	require.Equal(t, int64(5), counter.Value())
	require.Equal(t, uint64(2), counter.ClientVersion())
}

func TestInvalidCollectionName(t *testing.T) {
	_, err := NewClient("9starts-with-digit", "invalid")
	require.Error(t, err)
	require.ErrorIs(t, err, &Error{Code: ErrCodeInvalidCollectionName})
}

func TestReadonlyCounterRejectsWrites(t *testing.T) {
	client, err := NewClient("go-binding-test", "readonly")
	require.NoError(t, err)
	defer client.Close()

	counter, err := client.CreateCounter("ro-counter", &DatatypeOptions{Readonly: true})
	require.NoError(t, err)
	defer counter.Close()

	_, err = counter.Increase()
	require.Error(t, err)
}

func TestSyncBetweenTwoClients(t *testing.T) {
	conn := NewLocalConnectivity()
	defer conn.Close()
	conn.SetRealtime(false)

	client1, err := NewClient("go-binding-test", "sync-a", WithLocalConnectivity(conn))
	require.NoError(t, err)
	defer client1.Close()
	client2, err := NewClient("go-binding-test", "sync-b", WithLocalConnectivity(conn))
	require.NoError(t, err)
	defer client2.Close()

	counter1, err := client1.CreateCounter("shared-counter", nil)
	require.NoError(t, err)
	defer counter1.Close()

	_, err = counter1.Increase()
	require.NoError(t, err)
	require.NoError(t, counter1.Sync())
	require.Equal(t, StateSubscribed, counter1.State())

	counter2, err := client2.SubscribeCounter("shared-counter", nil)
	require.NoError(t, err)
	defer counter2.Close()

	require.NoError(t, counter2.Sync())
	require.Equal(t, int64(1), counter2.Value())
}

func TestTransactionCommitAndRollback(t *testing.T) {
	client, err := NewClient("go-binding-test", "transaction")
	require.NoError(t, err)
	defer client.Close()

	counter, err := client.CreateCounter("tx-counter", nil)
	require.NoError(t, err)
	defer counter.Close()

	err = counter.Transaction("commit", func(tx *Counter) error {
		if _, err := tx.IncreaseBy(10); err != nil {
			return err
		}
		if _, err := tx.IncreaseBy(5); err != nil {
			return err
		}
		return nil
	})
	require.NoError(t, err)
	require.Equal(t, int64(15), counter.Value())

	wantErr := fmt.Errorf("intentional failure")
	err = counter.Transaction("rollback", func(tx *Counter) error {
		if _, err := tx.IncreaseBy(100); err != nil {
			return err
		}
		return wantErr
	})
	require.ErrorIs(t, err, wantErr)
	require.Equal(t, int64(15), counter.Value())

	err = counter.Transaction("panic", func(tx *Counter) error {
		if _, err := tx.IncreaseBy(100); err != nil {
			return err
		}
		panic("boom")
	})
	require.Error(t, err)
	require.Equal(t, int64(15), counter.Value())
}

func TestHandlerOnStateChange(t *testing.T) {
	conn := NewLocalConnectivity()
	defer conn.Close()
	conn.SetRealtime(false)

	client, err := NewClient("go-binding-test", "handler", WithLocalConnectivity(conn))
	require.NoError(t, err)
	defer client.Close()

	type transition struct{ oldState, newState DatatypeState }
	changes := make(chan transition, 8)

	counter, err := client.CreateCounter("handler-counter", &DatatypeOptions{
		Handler: &Handler{
			OnStateChange: func(oldState, newState DatatypeState) {
				changes <- transition{oldState, newState}
			},
		},
	})
	require.NoError(t, err)
	defer counter.Close()

	_, err = counter.Increase()
	require.NoError(t, err)
	require.NoError(t, counter.Sync())

	select {
	case tr := <-changes:
		require.Equal(t, StateCreating, tr.oldState)
		require.Equal(t, StateSubscribed, tr.newState)
	case <-time.After(5 * time.Second):
		require.FailNow(t, "timed out waiting for OnStateChange")
	}
}

func TestSetAndUnsetHandler(t *testing.T) {
	client, err := NewClient("go-binding-test", "set-handler")
	require.NoError(t, err)
	defer client.Close()

	counter, err := client.CreateCounter("set-handler-counter", nil)
	require.NoError(t, err)
	defer counter.Close()

	counter.SetHandler(1, &Handler{
		OnStateChange: func(oldState, newState DatatypeState) {},
	})
	require.True(t, counter.UnsetHandler(1))
	require.False(t, counter.UnsetHandler(1))
}
