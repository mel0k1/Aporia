// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <string.h>
#include <sys/mman.h>
#include <sys/uio.h>
#include <sys/wait.h>
#include <unistd.h>

#define PAGE_SIZE 4096

FN_TEST(process_vm_readv_writev)
{
	// The shared mapping has the same address in both processes after
	// fork, so the parent knows the remote address without /proc/pid/maps.
	char *shared = mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
			    MAP_SHARED | MAP_ANONYMOUS, -1, 0);
	TEST_SUCC(shared);
	memset(shared, 0, PAGE_SIZE);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		// Wait for the parent to write the message into the shared
		// memory through process_vm_writev.
		while (strncmp(shared, "FROM_PARENT", 11) != 0) {
			usleep(1000);
		}

		// Reply in another buffer of the shared mapping.
		memcpy(shared + PAGE_SIZE / 2, "FROM_CHILD", 10);
		_exit(0);
	}

	// process_vm_writev writes the local data into the child's memory.
	struct iovec local_iov = { .iov_base = "FROM_PARENT", .iov_len = 11 };
	struct iovec remote_iov = { .iov_base = shared, .iov_len = 11 };
	TEST_RES(process_vm_writev(pid, &local_iov, 1, &remote_iov, 1, 0),
		 _ret == 11);

	// process_vm_readv reads the child's reply into a local buffer.
	char rbuf[16] = {};
	struct iovec local_read = { .iov_base = rbuf, .iov_len = 10 };
	struct iovec remote_read = {
		.iov_base = shared + PAGE_SIZE / 2,
		.iov_len = 10,
	};
	while (process_vm_readv(pid, &local_read, 1, &remote_read, 1, 0) !=
	       10) {
		usleep(1000);
	}
	TEST_RES(memcmp(rbuf, "FROM_CHILD", 10), _ret == 0);

	int status;
	TEST_SUCC(waitpid(pid, &status, 0));
	TEST_RES(WIFEXITED(status) && WEXITSTATUS(status) == 0, _ret);
}
END_TEST()

FN_TEST(process_vm_err)
{
	pid_t pid = TEST_SUCC(getpid());

	char good_buf[4];
	struct iovec iov = { .iov_base = good_buf, .iov_len = 4 };

	// Unknown process.
	TEST_ERRNO(process_vm_readv(0x7FFFFFFF, &iov, 1, &iov, 1, 0), ESRCH);
	// Unknown flags are not allowed.
	TEST_ERRNO(process_vm_readv(pid, &iov, 1, &iov, 1, 1), EINVAL);
	// Too many I/O vectors are not allowed.
	TEST_ERRNO(process_vm_readv(pid, &iov, 1025, &iov, 1, 0), EINVAL);
	// Negative buffer lengths are not allowed.
	struct iovec bad_len = { .iov_base = iov.iov_base, .iov_len = -1 };
	TEST_ERRNO(process_vm_readv(pid, &iov, 1, &bad_len, 1, 0), EINVAL);
	// Invalid local buffers are rejected.
	struct iovec bad_local = { .iov_base = (void *)-1, .iov_len = 4 };
	TEST_ERRNO(process_vm_readv(pid, &bad_local, 1, &iov, 1, 0), EFAULT);
	// Unmapped remote memory fails with EIO.
	struct iovec bad_remote = { .iov_base = (void *)PAGE_SIZE,
				    .iov_len = 4 };
	TEST_ERRNO(process_vm_readv(pid, &iov, 1, &bad_remote, 1, 0), EIO);
	// Writing to unmapped remote memory fails with EIO as well.
	TEST_ERRNO(process_vm_writev(pid, &iov, 1, &bad_remote, 1, 0), EIO);
}
END_TEST()

FN_TEST(process_vm_self)
{
	pid_t pid = TEST_SUCC(getpid());

	char buf[16] = {};
	struct iovec local_iov = { .iov_base = buf, .iov_len = 4 };
	struct iovec remote_iov = { .iov_base = "SELF", .iov_len = 4 };
	TEST_RES(process_vm_readv(pid, &local_iov, 1, &remote_iov, 1, 0),
		 _ret == 4);
	TEST_RES(memcmp(buf, "SELF", 4), _ret == 0);
}
END_TEST()
