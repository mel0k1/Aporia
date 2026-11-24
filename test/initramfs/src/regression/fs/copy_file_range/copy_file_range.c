#define _GNU_SOURCE

#include <fcntl.h>
#include <unistd.h>

#include "../../common/test.h"

static void write_pattern_fd(int fd, const char *pattern, int len)
{
	CHECK(lseek(fd, 0, SEEK_SET));
	CHECK(write(fd, pattern, len));
}

FN_TEST(copy_file_range_full_copy)
{
	const char *path_in = "/tmp/cfr_in";
	const char *path_out = "/tmp/cfr_out";
	char buf[8];
	int fd_in = CHECK(open(path_in, O_RDWR | O_CREAT | O_TRUNC, 0600));
	int fd_out = CHECK(open(path_out, O_RDWR | O_CREAT | O_TRUNC, 0600));

	write_pattern_fd(fd_in, "abcdefg", 7);

	TEST_RES(copy_file_range(fd_in, NULL, fd_out, NULL, 7, 0), _ret == 7);

	CHECK(lseek(fd_out, 0, SEEK_SET));
	TEST_RES(read(fd_out, buf, sizeof(buf)), _ret == 7);
	CHECK(memcmp(buf, "abcdefg", 7) == 0);

	CHECK(close(fd_in));
	CHECK(close(fd_out));
	CHECK(unlink(path_in));
	CHECK(unlink(path_out));
}
END_TEST()

FN_TEST(copy_file_range_explicit_offsets)
{
	const char *path_in = "/tmp/cfr_in2";
	const char *path_out = "/tmp/cfr_out2";
	off_t off_in = 2;
	off_t off_out = 1;
	char buf[8];
	int fd_in = CHECK(open(path_in, O_RDWR | O_CREAT | O_TRUNC, 0600));
	int fd_out = CHECK(open(path_out, O_RDWR | O_CREAT | O_TRUNC, 0600));

	write_pattern_fd(fd_in, "abcdefg", 7);

	TEST_RES(copy_file_range(fd_in, &off_in, fd_out, &off_out, 5, 0),
		 _ret == 5);
	CHECK(off_in == 7);
	CHECK(off_out == 6);

	CHECK(lseek(fd_out, 0, SEEK_SET));
	TEST_RES(read(fd_out, buf, sizeof(buf)), _ret == 6);
	CHECK(memcmp(buf, "bcdef", 5) == 0);

	CHECK(close(fd_in));
	CHECK(close(fd_out));
	CHECK(unlink(path_in));
	CHECK(unlink(path_out));
}
END_TEST()

FN_TEST(copy_file_range_partial_copy)
{
	const char *path_in = "/tmp/cfr_in3";
	const char *path_out = "/tmp/cfr_out3";
	char buf[8];
	int fd_in = CHECK(open(path_in, O_RDWR | O_CREAT | O_TRUNC, 0600));
	int fd_out = CHECK(open(path_out, O_RDWR | O_CREAT | O_TRUNC, 0600));

	write_pattern_fd(fd_in, "abc", 3);

	TEST_RES(copy_file_range(fd_in, NULL, fd_out, NULL, 10, 0), _ret == 3);

	CHECK(lseek(fd_out, 0, SEEK_SET));
	TEST_RES(read(fd_out, buf, sizeof(buf)), _ret == 3);
	CHECK(memcmp(buf, "abc", 3) == 0);

	CHECK(close(fd_in));
	CHECK(close(fd_out));
	CHECK(unlink(path_in));
	CHECK(unlink(path_out));
}
END_TEST()

FN_TEST(copy_file_range_err)
{
	const char *path = "/tmp/cfr_err";
	off_t negative = -1;
	off_t off_in = 0;
	off_t off_out = 4;
	int fd = CHECK(open(path, O_RDWR | O_CREAT | O_TRUNC, 0600));

	TEST_ERRNO(copy_file_range(fd, NULL, fd, NULL, 8, 1), EINVAL);
	TEST_ERRNO(copy_file_range(-1, NULL, fd, NULL, 8, 0), EBADF);
	TEST_ERRNO(copy_file_range(fd, &negative, fd, NULL, 8, 0), EINVAL);

	write_pattern_fd(fd, "abcd", 4);
	TEST_RES(copy_file_range(fd, &off_in, fd, &off_out, 4, 0), _ret == 4);
	CHECK(off_in == 4);
	CHECK(off_out == 8);

	off_in = 0;
	off_out = 2;
	TEST_ERRNO(copy_file_range(fd, &off_in, fd, &off_out, 4, 0), EINVAL);

	CHECK(close(fd));
	CHECK(unlink(path));
}
END_TEST()
