#define _GNU_SOURCE

#include <arpa/inet.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../common/test.h"

#define PAYLOAD_A "alpha"
#define PAYLOAD_B "beta"

FN_TEST(recvmmsg_udp_loopback)
{
	int sock = CHECK(socket(AF_INET, SOCK_DGRAM, 0));
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	CHECK(bind(sock, (struct sockaddr *)&addr, sizeof(addr)));
	CHECK(getsockname(sock, (struct sockaddr *)&addr, &addr_len));

	int sender = CHECK(socket(AF_INET, SOCK_DGRAM, 0));
	struct sockaddr_in dest = {
		.sin_family = AF_INET,
		.sin_addr.s_addr = htonl(INADDR_LOOPBACK),
		.sin_port = addr.sin_port,
	};
	CHECK(sendto(sender, PAYLOAD_A, strlen(PAYLOAD_A), 0,
		     (struct sockaddr *)&dest, sizeof(dest)));
	CHECK(sendto(sender, PAYLOAD_B, strlen(PAYLOAD_B), 0,
		     (struct sockaddr *)&dest, sizeof(dest)));

	char buf_a[16];
	char buf_b[16];
	struct iovec iov_a = { .iov_base = buf_a, .iov_len = sizeof(buf_a) };
	struct iovec iov_b = { .iov_base = buf_b, .iov_len = sizeof(buf_b) };
	struct mmsghdr msgs[2] = { 0 };
	msgs[0].msg_hdr.msg_iov = &iov_a;
	msgs[0].msg_hdr.msg_iovlen = 1;
	msgs[1].msg_hdr.msg_iov = &iov_b;
	msgs[1].msg_hdr.msg_iovlen = 1;

	TEST_RES(syscall(SYS_recvmmsg, sock, msgs, 2, 0, NULL), _ret == 2);
	CHECK(msgs[0].msg_len == strlen(PAYLOAD_A));
	CHECK(msgs[1].msg_len == strlen(PAYLOAD_B));
	CHECK(memcmp(buf_a, PAYLOAD_A, strlen(PAYLOAD_A)) == 0);
	CHECK(memcmp(buf_b, PAYLOAD_B, strlen(PAYLOAD_B)) == 0);

	CHECK(close(sender));
	CHECK(close(sock));
}
END_TEST()

FN_TEST(recvmmsg_err)
{
	char buf[16];
	struct iovec iov = { .iov_base = buf, .iov_len = sizeof(buf) };
	struct mmsghdr msgs[1] = { 0 };
	msgs[0].msg_hdr.msg_iov = &iov;
	msgs[0].msg_hdr.msg_iovlen = 1;

	TEST_ERRNO(syscall(SYS_recvmmsg, -1, msgs, 1, 0, NULL), EBADF);
}
END_TEST()
