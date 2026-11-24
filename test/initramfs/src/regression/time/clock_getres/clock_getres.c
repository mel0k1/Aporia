#define _GNU_SOURCE

#include <errno.h>
#include <time.h>

#include "../../common/test.h"

FN_TEST(clock_getres_high_res)
{
	struct timespec res = { 0 };
	TEST_RES(clock_getres(CLOCK_REALTIME, &res), _ret == 0);
	CHECK(res.tv_sec == 0);
	CHECK(res.tv_nsec > 0);

	TEST_RES(clock_getres(CLOCK_MONOTONIC, &res), _ret == 0);
	CHECK(res.tv_sec == 0);
	CHECK(res.tv_nsec > 0);

	TEST_RES(clock_getres(CLOCK_MONOTONIC_RAW, &res), _ret == 0);
	CHECK(res.tv_sec == 0);
	CHECK(res.tv_nsec > 0);

	TEST_RES(clock_getres(CLOCK_BOOTTIME, &res), _ret == 0);
	CHECK(res.tv_sec == 0);
	CHECK(res.tv_nsec > 0);
}
END_TEST()

FN_TEST(clock_getres_coarse)
{
	struct timespec res = { 0 };
	TEST_RES(clock_getres(CLOCK_REALTIME_COARSE, &res), _ret == 0);
	CHECK(res.tv_nsec > 0);
	CHECK(res.tv_sec == 0);

	TEST_RES(clock_getres(CLOCK_MONOTONIC_COARSE, &res), _ret == 0);
	CHECK(res.tv_nsec > 0);
	CHECK(res.tv_sec == 0);
}
END_TEST()

FN_TEST(clock_getres_null_and_err)
{
	struct timespec res = { 0 };
	int bad_thread_clock = ~(1000000 << 3 | 1);

	TEST_SUCC(clock_getres(CLOCK_REALTIME, NULL));
	TEST_ERRNO(clock_getres(-1, &res), EINVAL);
	TEST_ERRNO(clock_getres(bad_thread_clock, &res), EINVAL);
}
END_TEST()
