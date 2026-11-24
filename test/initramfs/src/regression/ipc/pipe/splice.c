// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <fcntl.h>
#include <signal.h>
#include <string.h>
#include <sys/uio.h>
#include <unistd.h>

#define PAGE_SIZE 4096
const char *filename = "testfile";

FN_SETUP()
{
	signal(SIGPIPE, SIG_IGN);
}
END_SETUP()

FN_TEST(splice_file_to_pipe)
{
	int fds[2];
	TEST_SUCC(pipe(fds));

	int fd = TEST_SUCC(open(filename, O_RDWR | O_CREAT | O_TRUNC, 0600));
	TEST_SUCC(ftruncate(fd, 2 * PAGE_SIZE));
	TEST_SUCC(write(fd, "HELLO", 5));

	// Splice from the file at its current offset.
	TEST_RES(splice(fd, NULL, fds[1], NULL, PAGE_SIZE, 0), _ret == 5);

	char buf[16] = {};
	TEST_RES(read(fds[0], buf, sizeof(buf)), _ret == 5);
	TEST_RES(memcmp(buf, "HELLO", 5), _ret == 0);

	// Splice with an explicit offset.
	off_t off = 0;
	TEST_RES(splice(fd, &off, fds[1], NULL, PAGE_SIZE, 0), _ret == 5);
	TEST_RES(off, off == 5);

	TEST_RES(read(fds[0], buf, sizeof(buf)), _ret == 5);
	TEST_RES(memcmp(buf, "HELLO", 5), _ret == 0);

	// A pipe cannot be used together with an explicit offset.
	off = 0;
	TEST_ERRNO(splice(fds[0], &off, fds[1], NULL, PAGE_SIZE, 0), ESPIPE);

	// Unknown flags must be rejected.
	TEST_ERRNO(splice(fd, NULL, fds[1], NULL, PAGE_SIZE, 0x10), EINVAL);

	TEST_SUCC(close(fd));
	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
	TEST_SUCC(unlink(filename));
}
END_TEST()

FN_TEST(splice_pipe_to_file)
{
	int fds[2];
	TEST_SUCC(pipe(fds));
	TEST_SUCC(write(fds[1], "WORLD", 5));

	int fd = TEST_SUCC(open(filename, O_RDWR | O_CREAT | O_TRUNC, 0600));

	TEST_RES(splice(fds[0], NULL, fd, NULL, PAGE_SIZE, 0), _ret == 5);

	char buf[16] = {};
	TEST_RES(pread(fd, buf, sizeof(buf), 0), _ret == 5);
	TEST_RES(memcmp(buf, "WORLD", 5), _ret == 0);
	// The file offset of fd is advanced by the spliced bytes.
	TEST_RES(lseek(fd, 0, SEEK_CUR), _ret == 5);

	TEST_SUCC(close(fd));
	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
	TEST_SUCC(unlink(filename));
}
END_TEST()

FN_TEST(splice_pipe_to_pipe)
{
	int fds1[2], fds2[2];
	TEST_SUCC(pipe(fds1));
	TEST_SUCC(pipe(fds2));
	TEST_SUCC(write(fds1[1], "ABC", 3));

	TEST_RES(splice(fds1[0], NULL, fds2[1], NULL, 10, 0), _ret == 3);

	// The source pipe is drained.
	char buf[16] = {};
	TEST_RES(read(fds1[0], buf, sizeof(buf)), _ret == 0);
	TEST_RES(read(fds2[0], buf, sizeof(buf)), _ret == 3);
	TEST_RES(memcmp(buf, "ABC", 3), _ret == 0);

	TEST_SUCC(close(fds1[0]));
	TEST_SUCC(close(fds1[1]));
	TEST_SUCC(close(fds2[0]));
	TEST_SUCC(close(fds2[1]));
}
END_TEST()

FN_TEST(vmsplice)
{
	int fds[2];
	TEST_SUCC(pipe(fds));

	char data[4] = "ABCD";
	struct iovec iov[2] = {
		{ .iov_base = data, .iov_len = 2 },
		{ .iov_base = data + 2, .iov_len = 2 },
	};
	TEST_RES(vmsplice(fds[1], iov, 2, 0), _ret == 4);

	char buf[16] = {};
	TEST_RES(read(fds[0], buf, sizeof(buf)), _ret == 4);
	TEST_RES(memcmp(buf, "ABCD", 4), _ret == 0);

	// vmsplice requires the file descriptor to refer to a pipe.
	TEST_ERRNO(vmsplice(STDOUT_FILENO, iov, 2, 0), EINVAL);
	// The pipe must be opened for writing.
	TEST_ERRNO(vmsplice(fds[0], iov, 2, 0), EBADF);
	// Unknown flags must be rejected.
	TEST_ERRNO(vmsplice(fds[1], iov, 2, 0x10), EINVAL);

	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
}
END_TEST()

FN_TEST(tee)
{
	int fds1[2], fds2[2];
	TEST_SUCC(pipe(fds1));
	TEST_SUCC(pipe(fds2));
	TEST_SUCC(write(fds1[1], "XYZ", 3));

	// tee duplicates the data without consuming the source pipe.
	TEST_RES(tee(fds1[0], fds2[1], 10, 0), _ret == 3);

	char buf[16] = {};
	TEST_RES(read(fds1[0], buf, sizeof(buf)), _ret == 3);
	TEST_RES(memcmp(buf, "XYZ", 3), _ret == 0);

	TEST_RES(read(fds2[0], buf, sizeof(buf)), _ret == 3);
	TEST_RES(memcmp(buf, "XYZ", 3), _ret == 0);

	// TEE-ing from an empty pipe fails with EAGAIN in non-blocking mode.
	TEST_ERRNO(tee(fds1[0], fds2[1], 10, SPLICE_F_NONBLOCK), EAGAIN);
	// Zero length is a no-op.
	TEST_RES(tee(fds1[0], fds2[1], 0, 0), _ret == 0);
	// Both file descriptors must refer to pipes.
	TEST_ERRNO(tee(fds1[0], STDOUT_FILENO, 10, 0), EINVAL);
	// Unknown flags must be rejected.
	TEST_ERRNO(tee(fds1[0], fds2[1], 10, 0x10), EINVAL);

	TEST_SUCC(close(fds1[0]));
	TEST_SUCC(close(fds1[1]));
	TEST_SUCC(close(fds2[0]));
	TEST_SUCC(close(fds2[1]));
}
END_TEST()
