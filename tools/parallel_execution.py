#!/usr/bin/env python3

import subprocess
from multiprocessing import Pool, Value
from datetime import datetime
from time import sleep
from sys import argv

num_complete = Value('i', 0)


def run_test(test, invocation_number, num_executions, stagger):
    global num_complete

    run_test_command = ["cargo", "test", test]

    if stagger > 0:
        sleep(stagger)

    start_time = datetime.now()

    # Launch a new test
    try:
        result = subprocess.Popen(run_test_command, stdout=subprocess.PIPE)
    except OSError:
        print("Invocation {} failed to launch the test".format(
            invocation_number))
        return False

    # Wait for the test to finish
    stdout = result.communicate()[0]

    elapsed_str = "in {}".format(datetime.now() - start_time)

    with num_complete.get_lock():
        success = result.returncode == 0
        if not success:
            filename = "failure_{}.txt".format(num_complete.value)
            print("{}/{} failed {}. Writing result to {}".format(
                num_complete.value + 1, num_executions, elapsed_str, filename))
            with open(filename, "w") as err_log:
                err_log.write(stdout.decode('utf-8'))
        else:
            print("{}/{} executed successfully {}".format(
                num_complete.value + 1,
                num_executions,
                elapsed_str))

        num_complete.value += 1
    return success


def run_tests(test='', num_executions=500, num_concurrent=8, total_stagger=0):
    with Pool(processes=num_concurrent) as pool:
        pauses = [total_stagger * (i / num_concurrent) *
                  (1 if i < num_concurrent else 0)
                  for i in range(num_executions)]
        results = [pool.apply_async(run_test, args=(
            test, i, num_executions, pauses[i])) for i in range(num_executions)]
        num_successful = sum(1 if result.get() else 0 for result in results)
        print("{}/{} succeeded for test {} {}".format(
            num_successful, num_executions, test,
            ':)' if num_successful == num_executions else ':('))


if __name__ == '__main__':
    if len(argv) == 5:
        run_tests(str(argv[1]), int(argv[2]), int(argv[3]), float(argv[4]))
    elif len(argv) == 4:
        run_tests(str(argv[1]), int(argv[2]), int(argv[3]))
    elif len(argv) == 3:
        run_tests(str(argv[1]), int(argv[2]))
    elif len(argv) == 2:
        run_tests(str(argv[1]))
    elif len(argv) == 1:
        run_tests()
    else:
        print("Usage: ./parallel_execution.py "
              "TEST NUMBER_OF_EXECUTIONS NUM_CONCURRENT STAGGER_DURATION")
