for testname in benchmark_many_clients_strong_consistency_simple benchmark_many_clients_strong_consistency_primary_fail benchmark_many_clients_strong_consistency_replica_fail benchmark_byzantine_hash_digest_mutlation_replica_simple benchmark_byzantine_commit_mutlation_replica_simple benchmark_byzantine_n_mutlation_replica_simple benchmark_byzantine_n_mutlation_replica_primary ; do
    echo $testname >> benchmark_output
    for i in `seq 1 10`; do
        cargo test --release $testname -- --nocapture 2>/dev/null | tee out | grep "Avg" >> benchmark_output
    done
done
