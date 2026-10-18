#import <Foundation/Foundation.h>
#import <Vision/Vision.h>
#import "mac_face.h"

MacFaceRect mac_detect_face_in_jpeg(const unsigned char *data, unsigned long len) {
    MacFaceRect result = { false, 0.0f, 0.0f, 0.0f, 0.0f };
    if (!data || len == 0) {
        return result;
    }

    @autoreleasepool {
        NSData *nsData = [NSData dataWithBytesNoCopy:(void*)data length:len freeWhenDone:NO];
        VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithData:nsData options:@{}];
        VNDetectFaceRectanglesRequest *request = [[VNDetectFaceRectanglesRequest alloc] init];

        NSError *error = nil;
        BOOL success = [handler performRequests:@[request] error:&error];
        if (!success || error) {
            return result;
        }

        NSArray *observations = request.results;
        if (!observations || observations.count == 0) {
            return result;
        }

        // Birden fazla yüz varsa en belirgin / en büyük alanı kaplayan yüzü seç
        VNFaceObservation *best = nil;
        double maxArea = 0.0;
        for (VNFaceObservation *obs in observations) {
            CGRect bbox = obs.boundingBox;
            double area = bbox.size.width * bbox.size.height;
            if (area > maxArea) {
                maxArea = area;
                best = obs;
            }
        }

        if (best) {
            CGRect bbox = best.boundingBox;
            // Vision Framework koordinat sistemi sol-alt orijinlidir (0.0..1.0).
            // Standart üst-sol kökene dönüştürmek için:
            // top_y = 1.0 - (origin.y + size.height)
            result.found = true;
            result.x = (float)bbox.origin.x;
            result.y = (float)(1.0 - (bbox.origin.y + bbox.size.height));
            result.width = (float)bbox.size.width;
            result.height = (float)bbox.size.height;
        }
    }

    return result;
}
