#define _GNU_SOURCE

#include <errno.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

struct robust_list {
	struct robust_list *next;
};

struct robust_list_head {
	struct robust_list *list;
	long futex_offset;
	struct robust_list *list_op_pending;
};

FN_TEST(get_robust_list_roundtrip)
{
	struct robust_list_head head = { 0 };
	struct robust_list_head got = { 0 };
	size_t len = 0;

	TEST_SUCC(syscall(SYS_set_robust_list, &head, sizeof(head)));
	TEST_RES(syscall(SYS_get_robust_list, 0, &got, &len), _ret == 0);
	CHECK(len == sizeof(head));
	CHECK(memcmp(&got, &head, sizeof(head)) == 0);
}
END_TEST()

FN_TEST(get_robust_list_err)
{
	struct robust_list_head head = { 0 };
	size_t len = 0;

	TEST_ERRNO(syscall(SYS_set_robust_list, &head, sizeof(head) - 1),
		   EINVAL);
	TEST_ERRNO(syscall(SYS_get_robust_list, 0, NULL, &len), EFAULT);
}
END_TEST()
