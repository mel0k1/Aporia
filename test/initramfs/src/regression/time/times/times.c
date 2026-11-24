#define _GNU_SOURCE

#include <sys/times.h>
#include <time.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(times_basic)
{
	struct tms tms = { 0 };
	clock_t ticks = TEST_SUCC(times(&tms));

	TEST_RES(times(NULL), _ret > 0);
	CHECK(tms.tms_utime >= 0);
	CHECK(tms.tms_stime >= 0);
	CHECK(ticks > 0);
}
END_TEST()
