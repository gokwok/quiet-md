// 编辑器里的原生玻璃。每个浮层独立滚动；更新选项时保留正在输入的控件与拼音组合串。
#import <UIKit/UIKit.h>
#import <ImageIO/ImageIO.h>
extern void quiet_editor_action(const char *, size_t, const char *);
// 行号按 TextKit 的实际行片段定位，折行和滚动都跟随正文。
@interface QuietSourceView : UITextView
@end
@implementation QuietSourceView
- (void)drawRect:(CGRect)rect {
    [super drawRect:rect];
    NSLayoutManager *lm = self.layoutManager;
    NSRange visible = [lm glyphRangeForBoundingRect:CGRectOffset(rect,0,-self.textContainerInset.top) inTextContainer:self.textContainer];
    if (!visible.length && self.text.length) return;
    NSString *text = self.text;
    NSUInteger start = visible.length ? [lm characterIndexForGlyphAtIndex:visible.location] : 0;
    __block NSUInteger line = 1;
    [text enumerateSubstringsInRange:NSMakeRange(0,start) options:NSStringEnumerationByLines|NSStringEnumerationSubstringNotRequired usingBlock:^(__unused NSString *s, __unused NSRange a, __unused NSRange b, __unused BOOL *stop) { line++; }];
    if (start && [text characterAtIndex:start-1] != '\n') line--;
    NSDictionary *attrs = @{NSFontAttributeName:[UIFont monospacedSystemFontOfSize:13 weight:UIFontWeightRegular],NSForegroundColorAttributeName:UIColor.tertiaryLabelColor};
    [lm enumerateLineFragmentsForGlyphRange:visible usingBlock:^(CGRect r, __unused CGRect used, __unused NSTextContainer *c, NSRange range, __unused BOOL *stop) {
        NSUInteger i = [lm characterIndexForGlyphAtIndex:range.location];
        if (!i || [text characterAtIndex:i-1]=='\n') {
            [[NSString stringWithFormat:@"%lu",(unsigned long)line++] drawAtPoint:CGPointMake(8,r.origin.y+self.textContainerInset.top) withAttributes:attrs];
        }
    }];
    if (lm.extraLineFragmentTextContainer) [[NSString stringWithFormat:@"%lu",(unsigned long)line] drawAtPoint:CGPointMake(8,lm.extraLineFragmentRect.origin.y+self.textContainerInset.top) withAttributes:attrs];
}
@end

void quiet_hide_keyboard(void) {
    [UIApplication.sharedApplication sendAction:@selector(resignFirstResponder) to:nil from:nil forEvent:nil];
}
// 附件缩略图：后台按目标尺寸解码，缓存后再贴回仍指向同一路径的图片视图。
static NSCache<NSString *, UIImage *> *thumbs;
static UIColor *hexColor(NSString *hex) {
    unsigned v = 0; [[NSScanner scannerWithString:[hex stringByReplacingOccurrencesOfString:@"#" withString:@""]] scanHexInt:&v];
    return [UIColor colorWithRed:((v>>16)&255)/255.0 green:((v>>8)&255)/255.0 blue:(v&255)/255.0 alpha:1];
}
static void thumbnail(UIImageView *view, NSString *path, CGFloat pixels) {
    if (!thumbs) thumbs = [NSCache new];
    view.accessibilityIdentifier = path; UIImage *hit = [thumbs objectForKey:path];
    view.image = hit; if (hit) return;
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED,0), ^{
        CGImageSourceRef src = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path], NULL);
        CGImageRef cg = src ? CGImageSourceCreateThumbnailAtIndex(src,0,(__bridge CFDictionaryRef)@{(id)kCGImageSourceCreateThumbnailFromImageAlways:@YES,
            (id)kCGImageSourceCreateThumbnailWithTransform:@YES,(id)kCGImageSourceThumbnailMaxPixelSize:@(pixels)}) : NULL;
        UIImage *image = cg ? [UIImage imageWithCGImage:cg] : nil;
        if (cg) CGImageRelease(cg); if (src) CFRelease(src);
        dispatch_async(dispatch_get_main_queue(), ^{
            if (image) [thumbs setObject:image forKey:path];
            if ([view.accessibilityIdentifier isEqual:path]) view.image = image;
        });
    });
}
// 文档面板（目录 / 任务 / 附件 / 查找）是同一张从底部升起的纸：切换分段时沿用同一个视图，只有首次出现和关闭时动画。
static BOOL sheetKey(NSString *key) { return [key isEqual:@"find"] || [key isEqual:@"catalogue"]; }
// 设置卡片（按钮、提醒、倒计时、编号、表格尺寸、表情）：底部一张玻璃卡片，整页连同顶栏压暗，点外面、往下拖都收起。同「文档信息」（`info.rs`）。
static BOOL cardKey(NSString *key) { return [@[@"button",@"reminder",@"countdown",@"number",@"size",@"emoji"] containsObject:key]; }
static UIColor *rgb(NSArray *c) { return [UIColor colorWithRed:[c[0] doubleValue] green:[c[1] doubleValue] blue:[c[2] doubleValue] alpha:c.count > 3 ? [c[3] doubleValue] : 1]; }
static void rule(UIView *row, CGFloat w, CGFloat h) {
    UIView *line = [[UIView alloc] initWithFrame:CGRectMake(0,h-1.0/3,w,1.0/3)]; line.backgroundColor = UIColor.separatorColor; [row addSubview:line];
}
// 表格尺寸：手指在格子上拖，划到哪格选到哪格，抬手插入；抬手时在外面就不插。
@interface QuietSizeGrid : UIView
@property NSInteger rows, cols, r, c;
@property UILabel *tally;
@property (copy) void (^picked)(NSInteger, NSInteger);
@end
@implementation QuietSizeGrid
- (CGFloat)pitch { return (self.bounds.size.width+6)/self.cols; }
- (void)drawRect:(CGRect)rect {
    CGFloat p = [self pitch];
    for (NSInteger i=0;i<self.rows;i++) for (NSInteger j=0;j<self.cols;j++) {
        [(i<self.r && j<self.c ? [self.tintColor colorWithAlphaComponent:0.75] : UIColor.tertiarySystemFillColor) setFill];
        [[UIBezierPath bezierPathWithRoundedRect:CGRectMake(j*p,i*p,p-6,p-6) cornerRadius:7] fill];
    }
}
- (void)pick:(UITouch *)touch {
    CGPoint at = [touch locationInView:self]; CGFloat p = [self pitch]; NSInteger r = 0, c = 0;
    if (CGRectContainsPoint(CGRectInset(self.bounds,-14,-14),at)) { r = MIN(self.rows,MAX(1,(NSInteger)(at.y/p)+1)); c = MIN(self.cols,MAX(1,(NSInteger)(at.x/p)+1)); }
    if (r == self.r && c == self.c) return;
    self.r = r; self.c = c; self.tally.text = r ? [NSString stringWithFormat:@"%ld x %ld",(long)c,(long)r] : @""; [self setNeedsDisplay];
}
- (void)touchesBegan:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event { [self pick:touches.anyObject]; }
- (void)touchesMoved:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event { [self pick:touches.anyObject]; }
- (void)touchesEnded:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event { [self pick:touches.anyObject]; if (self.r && self.picked) self.picked(self.r,self.c); }
- (void)touchesCancelled:(NSSet<UITouch *> *)touches withEvent:(UIEvent *)event { self.r = self.c = 0; self.tally.text = @""; [self setNeedsDisplay]; }
@end
// 表情：一格一个字的网格，分组标题吸顶；底下一排分类，跟着滚动走。
@interface QuietEmojiGrid : UIView <UICollectionViewDataSource, UICollectionViewDelegate>
@property UICollectionView *grid;
@property NSArray<NSDictionary *> *sections;
@property NSArray<NSString *> *symbols;
@property UIStackView *bar;
@property NSInteger current;
@property (copy) void (^picked)(NSInteger, NSInteger);
@end
@implementation QuietEmojiGrid
- (instancetype)initWithFrame:(CGRect)frame {
    self = [super initWithFrame:frame];
    UICollectionViewFlowLayout *flow = [UICollectionViewFlowLayout new]; flow.minimumInteritemSpacing = 0; flow.minimumLineSpacing = 0; flow.sectionHeadersPinToVisibleBounds = NO;
    self.grid = [[UICollectionView alloc] initWithFrame:CGRectZero collectionViewLayout:flow]; self.grid.backgroundColor = UIColor.clearColor;
    self.grid.dataSource = self; self.grid.delegate = self; self.grid.showsVerticalScrollIndicator = NO; self.grid.keyboardDismissMode = UIScrollViewKeyboardDismissModeOnDrag;
    self.grid.contentInsetAdjustmentBehavior = UIScrollViewContentInsetAdjustmentNever;
    [self.grid registerClass:UICollectionViewCell.class forCellWithReuseIdentifier:@"e"];
    [self.grid registerClass:UICollectionReusableView.class forSupplementaryViewOfKind:UICollectionElementKindSectionHeader withReuseIdentifier:@"h"];
    [self addSubview:self.grid];
    self.bar = [UIStackView new]; self.bar.distribution = UIStackViewDistributionFillEqually; [self addSubview:self.bar];
    return self;
}
- (void)layoutSubviews {
    [super layoutSubviews];
    BOOL bar = self.sections.count > 1; CGFloat foot = bar ? 44 : 0, w = self.bounds.size.width;
    self.bar.hidden = !bar; self.bar.frame = CGRectMake(0,self.bounds.size.height-foot,w,foot);
    self.grid.frame = CGRectMake(0,0,w,self.bounds.size.height-foot);
    CGFloat side = floor(w/8); ((UICollectionViewFlowLayout *)self.grid.collectionViewLayout).itemSize = CGSizeMake(side,side);
}
- (void)show:(NSArray *)sections symbols:(NSArray *)symbols {
    if ([self.sections isEqual:sections]) return;
    self.sections = sections; [self.grid reloadData]; self.grid.contentOffset = CGPointZero; self.current = 0;
    for (UIView *v in self.bar.arrangedSubviews.copy) [v removeFromSuperview];
    __weak QuietEmojiGrid *weak = self;
    if (sections.count > 1) for (NSUInteger i=0;i<symbols.count && i<sections.count;i++) {
        UIButton *b = [UIButton buttonWithType:UIButtonTypeSystem];
        [b setImage:[UIImage systemImageNamed:symbols[i] withConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:16 weight:UIImageSymbolWeightRegular]] forState:UIControlStateNormal];
        b.accessibilityLabel = sections[i][@"title"];
        [b addAction:[UIAction actionWithHandler:^(__unused UIAction *a) {
            CGRect head = [weak.grid layoutAttributesForSupplementaryElementOfKind:UICollectionElementKindSectionHeader atIndexPath:[NSIndexPath indexPathForItem:0 inSection:i]].frame;
            CGFloat end = MAX(0,weak.grid.contentSize.height-weak.grid.bounds.size.height);
            [weak.grid setContentOffset:CGPointMake(0,MIN(head.origin.y,end)) animated:YES];
        }] forControlEvents:UIControlEventTouchUpInside];
        [self.bar addArrangedSubview:b];
    }
    [self mark]; [self setNeedsLayout];
}
- (void)mark {
    [self.bar.arrangedSubviews enumerateObjectsUsingBlock:^(UIView *v, NSUInteger i, __unused BOOL *stop) { v.tintColor = (NSInteger)i == self.current ? UIColor.labelColor : UIColor.tertiaryLabelColor; }];
}
- (void)scrollViewDidScroll:(UIScrollView *)view {
    NSIndexPath *at = [self.grid indexPathForItemAtPoint:CGPointMake(8,view.contentOffset.y+40)];
    if (at && at.section != self.current) { self.current = at.section; [self mark]; }
}
- (NSInteger)numberOfSectionsInCollectionView:(UICollectionView *)v { return (NSInteger)self.sections.count; }
- (NSInteger)collectionView:(UICollectionView *)v numberOfItemsInSection:(NSInteger)s { return (NSInteger)[self.sections[s][@"items"] count]; }
- (CGSize)collectionView:(UICollectionView *)v layout:(UICollectionViewLayout *)l referenceSizeForHeaderInSection:(NSInteger)s { return CGSizeMake(v.bounds.size.width,[self.sections[s][@"title"] length] ? 32 : 4); }
- (UICollectionViewCell *)collectionView:(UICollectionView *)v cellForItemAtIndexPath:(NSIndexPath *)p {
    UICollectionViewCell *cell = [v dequeueReusableCellWithReuseIdentifier:@"e" forIndexPath:p];
    UILabel *label = [cell.contentView viewWithTag:7];
    if (!label) { label = [[UILabel alloc] initWithFrame:cell.contentView.bounds]; label.tag = 7; label.autoresizingMask = 18; label.font = [UIFont systemFontOfSize:28]; label.textAlignment = NSTextAlignmentCenter; [cell.contentView addSubview:label]; }
    label.text = self.sections[p.section][@"items"][p.item]; return cell;
}
- (UICollectionReusableView *)collectionView:(UICollectionView *)v viewForSupplementaryElementOfKind:(NSString *)kind atIndexPath:(NSIndexPath *)p {
    UICollectionReusableView *head = [v dequeueReusableSupplementaryViewOfKind:kind withReuseIdentifier:@"h" forIndexPath:p];
    UILabel *label = [head viewWithTag:7];
    if (!label) { label = [[UILabel alloc] initWithFrame:CGRectMake(6,0,v.bounds.size.width-12,32)]; label.tag = 7; label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleFootnote]; label.textColor = UIColor.secondaryLabelColor; [head addSubview:label]; }
    label.text = self.sections[p.section][@"title"]; return head;
}
- (void)collectionView:(UICollectionView *)v didSelectItemAtIndexPath:(NSIndexPath *)p { if (self.picked) self.picked(p.section,p.item); }
@end
@interface QuietEditorSurface : UIVisualEffectView <UIPickerViewDataSource, UIPickerViewDelegate, UITextFieldDelegate, UITextViewDelegate, UITableViewDataSource, UITableViewDelegate, UIGestureRecognizerDelegate>
@property NSString *key;
@property BOOL sheet;
@property BOOL card;
@property CGFloat inset;
@property BOOL pulled;
@property BOOL fromList;
@property BOOL adopted;
@property NSInteger tab;
@property UIPanGestureRecognizer *pull;
@property UIControl *backdrop;
@property UIScrollView *scroll;
@property NSArray<NSDictionary *> *rows;
@property NSMutableDictionary<NSString *, UIView *> *fields;
@property NSMutableDictionary<NSString *, NSDictionary *> *lastFields;
@property NSMutableArray<UIView *> *lines;
@property NSString *model;
@property UITableView *list;
@property NSInteger listStart;
@property NSInteger activeRow;
@property NSDictionary *firstOption;
@end

@implementation QuietEditorSurface
- (void)send:(NSNumber *)action value:(NSString *)value {
    if (action) quiet_editor_action(self.key.UTF8String, action.unsignedIntegerValue, value.UTF8String);
}
- (void)tappedBackdrop {
    [self send:self.rows.firstObject[@"outside"] value:@""];
}
- (NSDictionary *)rowFor:(UIView *)v {
    return v.tag >= 0 && v.tag < (NSInteger)self.rows.count ? self.rows[v.tag] : nil;
}
- (void)changed:(UITextField *)field {
    if (!field.markedTextRange) [self send:[self rowFor:field][@"input"] value:field.text];
}
- (void)textViewDidChange:(UITextView *)field {
    if (!field.markedTextRange) [self send:[self rowFor:field][@"input"] value:field.text];
}
- (BOOL)textView:(UITextView *)field shouldChangeTextInRange:(NSRange)range replacementText:(NSString *)text {
    if (![self.key isEqual:@"equation"]) return YES;
    NSString *old = field.text;
    if (NSMaxRange(range) > old.length) return NO;
    NSString *kept = [old stringByReplacingCharactersInRange:range withString:@""];
    NSUInteger count = [kept lengthOfBytesUsingEncoding:NSUTF32LittleEndianStringEncoding]/4;
    NSUInteger room = count < 10000 ? 10000-count : 0;
    NSData *replacement = [text dataUsingEncoding:NSUTF32LittleEndianStringEncoding];
    if (replacement.length/4 <= room) return YES;
    NSString *prefix = [[NSString alloc] initWithBytes:replacement.bytes length:room*4 encoding:NSUTF32LittleEndianStringEncoding];
    [field.textStorage replaceCharactersInRange:range withString:prefix];
    field.selectedRange = NSMakeRange(range.location+prefix.length,0);
    [self textViewDidChange:field]; return NO;
}
- (BOOL)textFieldShouldReturn:(UITextField *)field {
    [self send:[self rowFor:field][@"submit"] value:@""]; return NO;
}
- (UIView *)input:(NSDictionary *)r index:(NSInteger)i {
    NSString *key = r[@"field"];
    UIView *view = self.fields[key];
    if (!view) {
        if ([r[@"multi"] boolValue]) {
            UITextView *t = [[QuietSourceView alloc] initWithFrame:CGRectZero textContainer:nil]; t.textContainerInset = UIEdgeInsetsMake(8,36,12,10); t.delegate = self;
            t.font = [UIFont monospacedSystemFontOfSize:16 weight:UIFontWeightRegular]; t.text = r[@"value"]; view = t;
        } else {
            UITextField *t = [UITextField new]; t.delegate = self; t.placeholder = r[@"hint"] ?: key; t.text = r[@"value"];
            t.font = [UIFont preferredFontForTextStyle:UIFontTextStyleBody];
            t.leftView = [[UIView alloc] initWithFrame:CGRectMake(0,0,12,1)]; t.leftViewMode = UITextFieldViewModeAlways;
            if ([self.key isEqual:@"find"]) {
                UIImageView *glyph=[[UIImageView alloc] initWithImage:[UIImage systemImageNamed:[r[@"search"] boolValue] ? @"magnifyingglass" : @"arrow.turn.down.right"]];
                glyph.tintColor=UIColor.secondaryLabelColor; glyph.contentMode=UIViewContentModeCenter; glyph.frame=CGRectMake(4,0,32,36);
                UIView *lead=[[UIView alloc] initWithFrame:CGRectMake(0,0,38,36)]; [lead addSubview:glyph];
                t.leftView=lead; t.clearButtonMode=UITextFieldViewModeWhileEditing;
            }
            if (r[@"label"]) {
                // 卡片里的一行：左边是名字，右边输入，底下一道分隔线。
                UIView *lead = [[UIView alloc] initWithFrame:CGRectMake(0,0,104,52)]; UILabel *name = [[UILabel alloc] initWithFrame:CGRectMake(0,0,104,52)];
                name.text = r[@"label"]; name.font = [UIFont preferredFontForTextStyle:UIFontTextStyleBody]; [lead addSubview:name]; t.leftView = lead; t.clearButtonMode = UITextFieldViewModeWhileEditing;
            }
            [t addTarget:self action:@selector(changed:) forControlEvents:UIControlEventEditingChanged]; view = t;
        }
        view.accessibilityLabel = key;
        if ([r[@"multi"] boolValue]) view.accessibilityIdentifier=[@"quiet.field." stringByAppendingString:self.key];
        if ([self.key isEqual:@"link"] || [self.key isEqual:@"equation"] || [self.key isEqual:@"language"]) {
            id<UITextInputTraits> input = (id)view;
            input.autocapitalizationType = UITextAutocapitalizationTypeNone; input.autocorrectionType = UITextAutocorrectionTypeNo;
            input.smartQuotesType = UITextSmartQuotesTypeNo; input.smartDashesType = UITextSmartDashesTypeNo;
        }
        view.backgroundColor = [r[@"multi"] boolValue] ? UIColor.secondarySystemBackgroundColor : (self.card ? UIColor.quaternarySystemFillColor : UIColor.secondarySystemFillColor); view.layer.cornerRadius = 12;
        if (r[@"label"]) { view.backgroundColor = UIColor.clearColor; if ([r[@"rule"] boolValue]) rule(view,[r[@"width"] doubleValue],52); }
        if ([r[@"plain"] boolValue] || [r[@"url"] boolValue]) {
            UITextField *t = (id)view; t.autocapitalizationType = UITextAutocapitalizationTypeNone; t.autocorrectionType = UITextAutocorrectionTypeNo;
            t.smartQuotesType = UITextSmartQuotesTypeNo; t.smartDashesType = UITextSmartDashesTypeNo; if ([r[@"url"] boolValue]) t.keyboardType = UIKeyboardTypeURL;
        }
        self.fields[key] = view; [self.scroll addSubview:view];
        if ([r[@"focus"] boolValue]) { [view becomeFirstResponder];
            if ([self.key isEqual:@"equation"] || [self.key isEqual:@"find"] || [r[@"select"] boolValue]) [(UIResponder *)view selectAll:nil]; }
    }
    // 输入框里的字被编辑器改了（换「执行操作」时默认文字跟着换）：跟上。正在组字、自己刚打的不动。
    if (self.card && [view isKindOfClass:UITextField.class]) {
        UITextField *t = (id)view; NSDictionary *before = self.lastFields[key];
        if (before && ![before[@"value"] isEqual:r[@"value"]] && !t.markedTextRange && ![t.text isEqual:r[@"value"]]) t.text = r[@"value"];
        self.lastFields[key] = r;
    }
    view.tag = i; return view;
}
- (UIView<UITextInput> *)focusedInput {
    for (UIView *v in self.fields.allValues) if (v.isFirstResponder && [v conformsToProtocol:@protocol(UITextInput)]) return (id)v;
    return nil;
}
- (NSArray<UIKeyCommand *> *)keyCommands {
    UIView<UITextInput> *field = [self focusedInput];
    if (!field || field.markedTextRange) return @[];
    NSMutableArray *keys = [NSMutableArray arrayWithObject:UIKeyInputEscape];
    if ([self.key isEqual:@"language"]) [keys addObjectsFromArray:@[UIKeyInputUpArrow,UIKeyInputDownArrow]];
    if ([self.key isEqual:@"find"]) [keys addObject:@"\r"];
    if ([self.key isEqual:@"equation"] && field.selectedTextRange.empty) {
        if ([field comparePosition:field.selectedTextRange.start toPosition:field.beginningOfDocument] == NSOrderedSame) [keys addObject:UIKeyInputLeftArrow];
        if ([field comparePosition:field.selectedTextRange.end toPosition:field.endOfDocument] == NSOrderedSame) [keys addObject:UIKeyInputRightArrow];
    }
    NSMutableArray *commands = [NSMutableArray array];
    for (NSString *key in keys) {
        UIKeyCommand *c = [UIKeyCommand keyCommandWithInput:key modifierFlags:[key isEqual:@"\r"] ? UIKeyModifierShift : 0 action:@selector(fieldCommand:)];
        c.wantsPriorityOverSystemBehavior = YES; [commands addObject:c];
    }
    return commands;
}
- (void)fieldCommand:(UIKeyCommand *)command {
    NSDictionary *names = @{UIKeyInputEscape:@"escape",UIKeyInputUpArrow:@"up",UIKeyInputDownArrow:@"down",
        UIKeyInputLeftArrow:@"leftEdge",UIKeyInputRightArrow:@"rightEdge",@"\r":@"previous"};
    [self send:[self rowFor:[self focusedInput]][@"submit"] value:names[command.input]];
}
- (UIAction *)action:(NSDictionary *)item owner:(UIView *)owner {
    __weak QuietEditorSurface *weak = self;
    __weak UIView *source = owner;
    UIImage *image = [item[@"symbol"] length] ? [UIImage systemImageNamed:item[@"symbol"]] : nil;
    return [UIAction actionWithTitle:item[@"title"] image:image identifier:nil handler:^(__unused UIAction *a) {
        if (!weak || ![source isDescendantOfView:weak]) return;
        UIView<UITextInput> *field = [weak focusedInput];
        if (field.markedTextRange) {
            [field unmarkText];
            if ([field isKindOfClass:UITextField.class]) [weak changed:(id)field];
            else [weak textViewDidChange:(id)field];
        }
        [weak send:item[@"action"] value:@""];
    }];
}
- (UIMenu *)menu:(NSArray *)children owner:(UIView *)owner {
    // 同一 group 的选项放进一段，段与段之间是系统分隔线。
    NSMutableArray *sections = [NSMutableArray array], *actions = nil; id group = nil;
    for (NSDictionary *child in children) {
        if (!actions || !(child[@"group"] == group || [child[@"group"] isEqual:group])) { actions = [NSMutableArray array]; [sections addObject:actions]; group = child[@"group"]; }
        UIAction *a = [self action:child owner:owner]; if ([child[@"disabled"] boolValue]) a.attributes=UIMenuElementAttributesDisabled;
        a.state = [child[@"on"] boolValue] ? UIMenuElementStateOn : UIMenuElementStateOff; [actions addObject:a];
    }
    if (sections.count < 2) return [UIMenu menuWithChildren:sections.firstObject ?: @[]];
    NSMutableArray *menus = [NSMutableArray array];
    for (NSArray *part in sections) [menus addObject:[UIMenu menuWithTitle:@"" image:nil identifier:nil options:UIMenuOptionsDisplayInline children:part]];
    return [UIMenu menuWithChildren:menus];
}
- (UIView *)line:(NSArray *)items width:(CGFloat)width height:(CGFloat)height {
    UIScrollView *strip = [UIScrollView new]; strip.showsHorizontalScrollIndicator = NO;
    CGFloat x = 0;
    for (NSDictionary *item in items) {
        UIButtonConfiguration *c = UIButtonConfiguration.plainButtonConfiguration;
        NSString *symbol = item[@"symbol"], *title = item[@"title"];
        BOOL iconOnly = [item[@"iconOnly"] boolValue], single = items.count == 1, outline = [item[@"outline"] boolValue], on = [item[@"on"] boolValue];
        c.title = iconOnly ? nil : title;
        if ([item[@"label"] boolValue]) {
            UILabel *label = [UILabel new]; label.text = title; label.textColor=UIColor.secondaryLabelColor; label.font=[UIFont preferredFontForTextStyle:self.sheet ? UIFontTextStyleSubheadline : UIFontTextStyleFootnote];
            CGFloat span=[label sizeThatFits:CGSizeMake(width,height)].width+8; label.frame=CGRectMake(x+(self.sheet ? 8 : 4),0,span,height); [strip addSubview:label]; x+=span; continue;
        }
        if (symbol.length) c.image = [UIImage systemImageNamed:symbol];
        if ([item[@"match"] count]==2) {
            NSMutableAttributedString *snippet = [[NSMutableAttributedString alloc] initWithString:title attributes:@{NSFontAttributeName:[UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline]}];
            NSRange range = NSMakeRange([item[@"match"][0] unsignedIntegerValue],[item[@"match"][1] unsignedIntegerValue]);
            if (NSMaxRange(range)<=title.length) [snippet addAttribute:NSFontAttributeName value:[UIFont boldSystemFontOfSize:15] range:range];
            c.attributedTitle = snippet; c.titleLineBreakMode = NSLineBreakByTruncatingTail; c.background.backgroundColor = UIColor.secondarySystemBackgroundColor;
        }
        c.imagePadding = 8; c.cornerStyle = UIButtonConfigurationCornerStyleCapsule;
        c.contentInsets = NSDirectionalEdgeInsetsMake(0, 8, 0, 8);
        if ([item[@"stack"] boolValue]) { c.imagePlacement = NSDirectionalRectEdgeTop; c.imagePadding = 8; }
        if (on && !outline) c.background.backgroundColor = UIColor.tertiarySystemFillColor;
        c.baseForegroundColor = [item[@"danger"] boolValue] ? UIColor.systemRedColor : (on && !outline ? self.tintColor : UIColor.labelColor);
        if (outline) {
            // 目录、任务、附件行：正文字号，当前项加粗变深（底色由行本身画），其余略浅。
            UIFont *body = [UIFont preferredFontForTextStyle:UIFontTextStyleBody];
            UIFont *font = on ? [UIFont systemFontOfSize:body.pointSize weight:UIFontWeightSemibold] : body;
            c.titleTextAttributesTransformer = ^NSDictionary *(NSDictionary *attrs) { NSMutableDictionary *m = attrs.mutableCopy; m[NSFontAttributeName] = font; return m; };
            c.baseForegroundColor = on ? UIColor.labelColor : ([item[@"done"] boolValue] ? UIColor.tertiaryLabelColor : UIColor.secondaryLabelColor);
            UIColor *glyph = [item[@"done"] boolValue] ? self.tintColor : UIColor.tertiaryLabelColor;
            c.imageColorTransformer = ^UIColor *(__unused UIColor *color) { return glyph; };
            c.preferredSymbolConfigurationForImage = [UIImageSymbolConfiguration configurationWithPointSize:16 weight:UIImageSymbolWeightRegular];
            c.contentInsets = NSDirectionalEdgeInsetsMake(0, 12, 0, 8); c.imagePadding = 10;
        }
        if ([item[@"minor"] boolValue]) {
            c.preferredSymbolConfigurationForImage = [UIImageSymbolConfiguration configurationWithPointSize:13 weight:UIImageSymbolWeightSemibold];
            c.baseForegroundColor = UIColor.tertiaryLabelColor;
        }
        if ([self.key hasSuffix:@"bar"]) c.preferredSymbolConfigurationForImage = [UIImageSymbolConfiguration configurationWithPointSize:15 weight:UIImageSymbolWeightRegular];
        if ([item[@"muted"] boolValue]) c.baseForegroundColor = UIColor.secondaryLabelColor;
        if ([item[@"menuTitle"] boolValue]) {
            // 「查找 ⌄」：文字在前，小号下箭头跟在后面。
            c.imagePlacement = NSDirectionalRectEdgeTrailing; c.imagePadding = 4; c.baseForegroundColor = UIColor.secondaryLabelColor;
            c.preferredSymbolConfigurationForImage = [UIImageSymbolConfiguration configurationWithPointSize:11 weight:UIImageSymbolWeightSemibold];
            UIFont *font = [UIFont systemFontOfSize:[UIFont preferredFontForTextStyle:UIFontTextStyleBody].pointSize weight:UIFontWeightMedium];
            c.titleTextAttributesTransformer = ^NSDictionary *(NSDictionary *attrs) { NSMutableDictionary *m = attrs.mutableCopy; m[NSFontAttributeName] = font; return m; };
        }
        UIButton *b = [UIButton buttonWithConfiguration:c primaryAction:nil];
        if (item[@"swatch"]) {
            NSArray *rgba = item[@"swatch"];
            if (rgba.count == 4) {
                UIColor *color = [UIColor colorWithRed:[rgba[0] doubleValue] green:[rgba[1] doubleValue] blue:[rgba[2] doubleValue] alpha:[rgba[3] doubleValue]];
                c.image = [[UIImage systemImageNamed:@"circle.fill"] imageWithTintColor:color renderingMode:UIImageRenderingModeAlwaysOriginal];
            } else c.image = [UIImage systemImageNamed:@"circle.slash"];
            b.configuration = c;
        }
        b.accessibilityIdentifier = [NSString stringWithFormat:@"quiet.%@.%@",self.key,symbol.length ? symbol : title];
        b.accessibilityLabel = title; b.enabled = ![item[@"disabled"] boolValue];
        if (on) b.accessibilityTraits |= UIAccessibilityTraitSelected;
        if ([item[@"children"] count]) {
            b.menu = [self menu:item[@"children"] owner:b]; b.showsMenuAsPrimaryAction = YES;
        } else [b addAction:[self action:item owner:b] forControlEvents:UIControlEventTouchUpInside];
        CGFloat span = single ? width : MAX(44, iconOnly ? 44 : [b sizeThatFits:CGSizeMake(width,height)].width);
        if ([item[@"equal"] boolValue]) span = width/items.count;
        if ([item[@"trailing"] boolValue]) { if (single) span=44; x = MAX(x,width-span-[item[@"reserve"] doubleValue]); }
        if ([item[@"fill"] boolValue]) span = MAX(44,width-x-[item[@"reserve"] doubleValue]);
        b.titleLabel.lineBreakMode = NSLineBreakByTruncatingTail;
        if (single || [item[@"fill"] boolValue]) b.contentHorizontalAlignment = UIControlContentHorizontalAlignmentLeading;
        if ([self.key isEqual:@"selection"] && ( [symbol isEqual:@"plus.circle"] || [symbol isEqual:@"function"] )) b.contentHorizontalAlignment = UIControlContentHorizontalAlignmentCenter;
        b.frame = CGRectMake(x,0,span,[item[@"stack"] boolValue] ? 80 : height); [strip addSubview:b];
        x += span;
    }
    strip.contentSize = CGSizeMake(x,height); strip.scrollEnabled = x > width; return strip;
}
- (UIView *)documentHeader:(NSDictionary *)r width:(CGFloat)w height:(CGFloat)h {
    // 左边是这篇文档的小缩略图（和主页列表行首同一套，Rust 的 `mini_page` 算好每道线），有自己的图标时放图标；标题下一行写它所在的空间，右边灰色圆形关闭钮。
    UIView *header = [UIView new]; CGFloat mid = h/2;
    NSDictionary *t = r[@"thumb"]; NSString *emoji = [t[@"icon"] isKindOfClass:NSString.class] ? t[@"icon"] : nil;
    if (emoji.length) {
        UILabel *icon = [UILabel new]; icon.text = emoji; icon.font = [UIFont systemFontOfSize:26]; icon.textAlignment = NSTextAlignmentCenter;
        icon.frame = CGRectMake(4,mid-18,34,36); [header addSubview:icon];
    } else if (t) {
        CGFloat pw = [t[@"w"] doubleValue], ph = [t[@"h"] doubleValue];
        UIColor *ink = hexColor(t[@"ink"]);
        UIView *page = [[UIView alloc] initWithFrame:CGRectMake(21-pw/2,mid-ph/2,pw,ph)];
        page.backgroundColor = hexColor(t[@"paper"]); page.layer.cornerRadius = 4; page.layer.cornerCurve = kCACornerCurveContinuous; page.clipsToBounds = YES;
        page.layer.borderWidth = 0.5; page.layer.borderColor = UIColor.separatorColor.CGColor;
        for (NSDictionary *q in t[@"parts"]) {
            CGRect f = CGRectMake([q[@"x"] doubleValue],[q[@"y"] doubleValue],[q[@"w"] doubleValue],[q[@"h"] doubleValue]);
            UIColor *c = [(q[@"c"] ? hexColor(q[@"c"]) : ink) colorWithAlphaComponent:[q[@"a"] doubleValue]];
            UIView *v;
            if (q[@"img"]) { UIImageView *iv = [[UIImageView alloc] initWithFrame:f]; iv.contentMode = UIViewContentModeScaleAspectFill; iv.backgroundColor = c; thumbnail(iv,q[@"img"],MAX(f.size.width,f.size.height)*4); v = iv; }
            else { v = [[UIView alloc] initWithFrame:f]; if (q[@"stroke"]) { v.layer.borderWidth = [q[@"stroke"] doubleValue]; v.layer.borderColor = c.CGColor; } else v.backgroundColor = c; }
            v.layer.cornerRadius = [q[@"r"] doubleValue]; v.clipsToBounds = YES; [page addSubview:v];
        }
        [header addSubview:page];
    }
    UILabel *label = [UILabel new]; label.text = r[@"header"]; label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleHeadline];
    label.frame = CGRectMake(48,mid-21,w-100,22); [header addSubview:label];
    UILabel *space = [UILabel new]; space.text = r[@"space"]; space.font = [UIFont preferredFontForTextStyle:UIFontTextStyleFootnote];
    space.textColor = UIColor.secondaryLabelColor; space.frame = CGRectMake(48,mid+2,w-100,18); [header addSubview:space];
    NSDictionary *item = [r[@"items"] firstObject];
    UIButton *close = [UIButton buttonWithType:UIButtonTypeSystem];
    UIImageSymbolConfiguration *look = [[UIImageSymbolConfiguration configurationWithPointSize:28] configurationByApplyingConfiguration:
        [UIImageSymbolConfiguration configurationWithPaletteColors:@[UIColor.secondaryLabelColor,UIColor.tertiarySystemFillColor]]];
    [close setImage:[UIImage systemImageNamed:@"xmark.circle.fill" withConfiguration:look] forState:UIControlStateNormal];
    close.accessibilityIdentifier = [@"quiet.close." stringByAppendingString:self.key]; close.accessibilityLabel = item[@"title"];
    close.frame = CGRectMake(w-44,mid-22,44,44); [header addSubview:close];
    [close addAction:[self action:item owner:close] forControlEvents:UIControlEventTouchUpInside];
    return header;
}
- (NSDictionary *)rowNamed:(NSString *)name {
    for (NSDictionary *r in self.rows) if ([r[@"control"] isEqual:name]) return r;
    return nil;
}
- (UIButton *)text:(NSDictionary *)item {
    UIButton *b = [UIButton buttonWithType:UIButtonTypeSystem]; UIFont *body = [UIFont preferredFontForTextStyle:UIFontTextStyleBody];
    if ([item[@"symbol"] length]) [b setImage:[UIImage systemImageNamed:item[@"symbol"] withConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:17 weight:UIImageSymbolWeightRegular]] forState:UIControlStateNormal];
    else [b setTitle:item[@"title"] forState:UIControlStateNormal];
    b.titleLabel.font = [item[@"primary"] boolValue] ? [UIFont systemFontOfSize:body.pointSize weight:UIFontWeightSemibold] : body;
    b.tintColor = [item[@"primary"] boolValue] ? self.tintColor : UIColor.labelColor; b.enabled = ![item[@"disabled"] boolValue];
    b.accessibilityLabel = item[@"title"]; b.accessibilityIdentifier = [NSString stringWithFormat:@"quiet.%@.%@",self.key,item[@"title"]];
    [b addAction:[self action:item owner:b] forControlEvents:UIControlEventTouchUpInside]; return b;
}
- (UIDatePicker *)datePicker:(NSDictionary *)r style:(UIDatePickerStyle)style mode:(UIDatePickerMode)mode {
    UIDatePicker *p = [UIDatePicker new]; p.datePickerMode = mode; p.preferredDatePickerStyle = style;
    // 时间都按应用自己的时区算（`time::local_offset`），和正文里的提醒一致。
    NSTimeZone *zone = [NSTimeZone timeZoneForSecondsFromGMT:[r[@"zone"] integerValue]];
    NSCalendar *cal = [NSCalendar calendarWithIdentifier:NSCalendarIdentifierGregorian]; cal.locale = [NSLocale localeWithLocaleIdentifier:r[@"locale"]]; cal.timeZone = zone; cal.firstWeekday = 1;
    p.calendar = cal; p.locale = cal.locale; p.timeZone = zone; return p;
}
- (void)show:(UIDatePicker *)p at:(NSNumber *)unix {
    NSDate *date = [NSDate dateWithTimeIntervalSince1970:unix.doubleValue];
    if (fabs([p.date timeIntervalSinceDate:date]) >= 1) [p setDate:date animated:NO];
}
// 带状态的系统控件（日期、滚轮、开关、网格、表情）按名字留着：编辑器每改一次都会重发这些行，重建会打断正在滚、正在拖的手势。
- (UIView *)control:(NSDictionary *)r width:(CGFloat)w height:(CGFloat)h {
    NSString *name = r[@"control"], *kind = r[@"kind"]; UIView *v = self.fields[name]; BOOL fresh = !v; __weak QuietEditorSurface *weak = self;
    if ([kind isEqual:@"date"]) {
        UIDatePicker *p = (id)v;
        if (fresh) {
            p = [self datePicker:r style:UIDatePickerStyleInline mode:[r[@"clock"] boolValue] ? UIDatePickerModeDateAndTime : UIDatePickerModeDate]; v = p;
            [p addAction:[UIAction actionWithHandler:^(UIAction *a) { [weak send:[weak rowNamed:name][@"action"] value:[NSString stringWithFormat:@"%.0f",((UIDatePicker *)a.sender).date.timeIntervalSince1970]]; }] forControlEvents:UIControlEventValueChanged];
        }
        [self show:p at:r[@"value"]];
    } else if ([kind isEqual:@"wheels"]) {
        UIPickerView *p = (id)v; NSArray *values = r[@"value"];
        if (fresh) { p = [UIPickerView new]; p.layer.name = name; p.dataSource = self; p.delegate = self; v = p; }
        for (NSInteger i=0;i<(NSInteger)values.count;i++) if ([p selectedRowInComponent:i] != [values[i] integerValue]) [p selectRow:[values[i] integerValue] inComponent:i animated:NO];
    } else if ([kind isEqual:@"switch"]) {
        // 名字 + 开关；开着时开关左边还有一个时间（紧凑的系统时间钮）或一个下拉菜单。
        UILabel *label; UISwitch *sw; UIDatePicker *time; UIButton *menu;
        if (fresh) {
            v = [UIView new]; label = [UILabel new]; label.tag = 1; label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleBody]; label.adjustsFontSizeToFitWidth = YES; label.minimumScaleFactor = 0.8; [v addSubview:label];
            sw = [UISwitch new]; sw.tag = 2; [v addSubview:sw];
            [sw addAction:[UIAction actionWithHandler:^(UIAction *a) { [weak send:[weak rowNamed:name][@"toggle"] value:((UISwitch *)a.sender).on ? @"1" : @"0"]; }] forControlEvents:UIControlEventValueChanged];
        }
        label = [v viewWithTag:1]; sw = [v viewWithTag:2]; time = [v viewWithTag:3]; menu = [v viewWithTag:4];
        label.text = r[@"title"]; sw.accessibilityLabel = r[@"title"];
        CGSize s = sw.intrinsicContentSize; sw.frame = CGRectMake(w-s.width,(h-s.height)/2,s.width,s.height);
        if (sw.on != [r[@"on"] boolValue]) [sw setOn:[r[@"on"] boolValue] animated:!fresh];
        CGFloat right = w-s.width-10;
        if (r[@"time"]) {
            if (!time) {
                time = [self datePicker:r style:UIDatePickerStyleCompact mode:UIDatePickerModeTime]; time.tag = 3; [v addSubview:time];
                [time addAction:[UIAction actionWithHandler:^(UIAction *a) { [weak send:[weak rowNamed:name][@"action"] value:[NSString stringWithFormat:@"%.0f",((UIDatePicker *)a.sender).date.timeIntervalSince1970]]; }] forControlEvents:UIControlEventValueChanged];
            }
            [self show:time at:r[@"time"]]; CGSize t = [time sizeThatFits:CGSizeMake(140,h)]; time.frame = CGRectMake(right-t.width,(h-t.height)/2,t.width,t.height); right -= t.width;
        }
        time.hidden = !r[@"time"];
        if (r[@"children"]) {
            if (!menu) { menu = [UIButton buttonWithConfiguration:UIButtonConfiguration.plainButtonConfiguration primaryAction:nil]; menu.tag = 4; menu.showsMenuAsPrimaryAction = YES; [v addSubview:menu]; }
            [self value:menu row:r]; CGSize t = [menu sizeThatFits:CGSizeMake(220,h)]; menu.frame = CGRectMake(right-t.width,0,t.width,h); right -= t.width;
        }
        menu.hidden = !r[@"children"];
        label.frame = CGRectMake(0,0,MAX(60,right-8),h);
    } else if ([kind isEqual:@"size"]) {
        QuietSizeGrid *g;
        if (fresh) {
            v = [UIView new]; UILabel *title = [[UILabel alloc] initWithFrame:CGRectMake(2,0,w-70,44)]; title.text = r[@"title"]; title.font = [UIFont preferredFontForTextStyle:UIFontTextStyleHeadline]; [v addSubview:title];
            UILabel *count = [[UILabel alloc] initWithFrame:CGRectMake(w-72,0,70,44)]; count.textAlignment = NSTextAlignmentRight; count.textColor = UIColor.secondaryLabelColor; count.font = [UIFont monospacedDigitSystemFontOfSize:15 weight:UIFontWeightRegular]; [v addSubview:count];
            g = [[QuietSizeGrid alloc] initWithFrame:CGRectMake(0,48,w,w)]; g.backgroundColor = UIColor.clearColor; g.contentMode = UIViewContentModeRedraw; g.tally = count; g.rows = [r[@"rows"] integerValue]; g.cols = [r[@"cols"] integerValue]; g.tag = 5; [v addSubview:g];
            g.picked = ^(NSInteger rr, NSInteger cc) { [weak send:[weak rowNamed:name][@"action"] value:[NSString stringWithFormat:@"%ld,%ld",(long)rr,(long)cc]]; };
        }
    } else if ([kind isEqual:@"emoji"]) {
        QuietEmojiGrid *g = (id)v;
        if (fresh) { g = [[QuietEmojiGrid alloc] initWithFrame:CGRectMake(0,0,w,h)]; v = g; g.picked = ^(NSInteger s, NSInteger i) { [weak send:[weak rowNamed:name][@"action"] value:[NSString stringWithFormat:@"%ld,%ld",(long)s,(long)i]]; }; }
        [g show:r[@"sections"] symbols:r[@"symbols"]];
    }
    if (fresh) { v.accessibilityIdentifier = [NSString stringWithFormat:@"quiet.%@.%@",self.key,name]; self.fields[name] = v; [self.scroll addSubview:v]; if ([r[@"rule"] boolValue]) rule(v,w,h); }
    return v;
}
// 右边的取值：字 + 小号上下箭头，点了是系统菜单。
- (void)value:(UIButton *)b row:(NSDictionary *)r {
    UIButtonConfiguration *c = UIButtonConfiguration.plainButtonConfiguration; c.title = r[@"value"]; c.image = [UIImage systemImageNamed:@"chevron.up.chevron.down"];
    c.imagePlacement = NSDirectionalRectEdgeTrailing; c.imagePadding = 4; c.baseForegroundColor = UIColor.secondaryLabelColor; c.contentInsets = NSDirectionalEdgeInsetsMake(0,8,0,4);
    c.preferredSymbolConfigurationForImage = [UIImageSymbolConfiguration configurationWithPointSize:11 weight:UIImageSymbolWeightMedium]; c.titleLineBreakMode = NSLineBreakByTruncatingTail;
    b.configuration = c; b.menu = [self menu:r[@"children"] owner:b]; b.accessibilityLabel = [NSString stringWithFormat:@"%@ %@",r[@"title"],r[@"value"]];
}
- (NSInteger)numberOfComponentsInPickerView:(UIPickerView *)p { return (NSInteger)[[self rowNamed:p.layer.name][@"value"] count]; }
- (NSInteger)pickerView:(UIPickerView *)p numberOfRowsInComponent:(NSInteger)c { return [[self rowNamed:p.layer.name][@"max"][c] integerValue]+1; }
- (UIView *)pickerView:(UIPickerView *)p viewForRow:(NSInteger)row forComponent:(NSInteger)c reusingView:(UIView *)old {
    UILabel *label = [old isKindOfClass:UILabel.class] ? (id)old : [UILabel new]; NSString *unit = [self rowNamed:p.layer.name][@"units"][c];
    NSMutableAttributedString *s = [[NSMutableAttributedString alloc] initWithString:[NSString stringWithFormat:@"%ld",(long)row] attributes:@{NSFontAttributeName:[UIFont monospacedDigitSystemFontOfSize:21 weight:UIFontWeightRegular],NSForegroundColorAttributeName:UIColor.labelColor}];
    [s appendAttributedString:[[NSAttributedString alloc] initWithString:[@" " stringByAppendingString:unit] attributes:@{NSFontAttributeName:[UIFont preferredFontForTextStyle:UIFontTextStyleFootnote],NSForegroundColorAttributeName:UIColor.secondaryLabelColor}]];
    label.attributedText = s; label.textAlignment = NSTextAlignmentCenter; return label;
}
- (CGFloat)pickerView:(UIPickerView *)p rowHeightForComponent:(NSInteger)c { return 36; }
- (void)pickerView:(UIPickerView *)p didSelectRow:(NSInteger)row inComponent:(NSInteger)c {
    NSMutableArray *v = [NSMutableArray array]; for (NSInteger i=0;i<p.numberOfComponents;i++) [v addObject:@([p selectedRowInComponent:i]).stringValue];
    [self send:[self rowNamed:p.layer.name][@"action"] value:[v componentsJoinedByString:@","]];
}
- (UIView *)cardBar:(NSDictionary *)r width:(CGFloat)w height:(CGFloat)h {
    UIView *bar = [UIView new]; UILabel *title = [UILabel new]; title.text = r[@"bar"]; title.font = [UIFont preferredFontForTextStyle:UIFontTextStyleHeadline]; title.textAlignment = NSTextAlignmentCenter;
    title.frame = CGRectMake(76,0,w-152,h); [bar addSubview:title];
    for (NSDictionary *item in r[@"items"]) {
        UIButton *b = [self text:item]; CGFloat span = MAX(44,[b sizeThatFits:CGSizeMake(80,h)].width+4); BOOL right = [item[@"primary"] boolValue];
        b.frame = CGRectMake(right ? w-span : 0,0,span,h); b.contentHorizontalAlignment = right ? UIControlContentHorizontalAlignmentTrailing : UIControlContentHorizontalAlignmentLeading; [bar addSubview:b];
    }
    return bar;
}
- (UIView *)valueRow:(NSDictionary *)r width:(CGFloat)w height:(CGFloat)h {
    UIView *row = [UIView new]; UILabel *label = [[UILabel alloc] initWithFrame:CGRectMake(0,0,w/2,h)]; label.text = r[@"title"]; label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleBody]; [row addSubview:label];
    UIButton *b = [UIButton buttonWithConfiguration:UIButtonConfiguration.plainButtonConfiguration primaryAction:nil]; b.showsMenuAsPrimaryAction = YES; [self value:b row:r];
    CGFloat span = MIN(w*0.6,[b sizeThatFits:CGSizeMake(w*0.6,h)].width); b.frame = CGRectMake(w-span,0,span,h); [row addSubview:b];
    if ([r[@"rule"] boolValue]) rule(row,w,h);
    return row;
}
- (UIView *)swatches:(NSDictionary *)r width:(CGFloat)w height:(CGFloat)h {
    UIView *row = [UIView new]; NSArray *items = r[@"swatches"]; NSInteger per = 8; CGFloat pitch = w/per; __weak QuietEditorSurface *weak = self;
    for (NSInteger i=0;i<(NSInteger)items.count;i++) {
        BOOL on = [r[@"on"] integerValue] == i; UIButton *b = [UIButton buttonWithType:UIButtonTypeCustom]; b.frame = CGRectMake((i%per)*pitch,8+(i/per)*44,pitch,44);
        UIView *dot = [[UIView alloc] initWithFrame:CGRectMake(pitch/2-13,9,26,26)]; dot.userInteractionEnabled = NO; dot.backgroundColor = rgb(items[i]); dot.layer.cornerRadius = 13; dot.layer.borderWidth = 1.0/3; dot.layer.borderColor = UIColor.separatorColor.CGColor; [b addSubview:dot];
        if (on) { UIView *ring = [[UIView alloc] initWithFrame:CGRectMake(pitch/2-17,5,34,34)]; ring.userInteractionEnabled = NO; ring.layer.cornerRadius = 17; ring.layer.borderWidth = 2; ring.layer.borderColor = self.tintColor.CGColor; [b addSubview:ring]; b.accessibilityTraits |= UIAccessibilityTraitSelected; }
        b.accessibilityLabel = [NSString stringWithFormat:@"%@ %ld",r[@"title"],(long)i+1];
        [b addAction:[UIAction actionWithHandler:^(__unused UIAction *a) { [weak send:r[@"action"] value:@(i).stringValue]; }] forControlEvents:UIControlEventTouchUpInside]; [row addSubview:b];
    }
    if ([r[@"rule"] boolValue]) rule(row,w,h);
    return row;
}
- (void)update:(NSArray *)rows width:(CGFloat)w {
    NSArray *old = self.rows; self.rows = rows;
    CGFloat y = 6, x = self.inset;
    NSUInteger boundary = [rows indexOfObjectPassingTest:^BOOL(NSDictionary *r, __unused NSUInteger idx, __unused BOOL *stop) { return [r[@"list"] boolValue]; }];
    BOOL listed = boundary != NSNotFound, switched = NO;
    self.listStart = listed ? boundary+1 : rows.count;
    NSMutableSet *keep = [NSMutableSet set]; NSMutableArray *fresh = [NSMutableArray array]; BOOL below = NO;
    for (NSInteger i = 0; i < self.listStart; ++i) {
        NSDictionary *r = rows[i];
        CGFloat h = r[@"h"] ? [r[@"h"] doubleValue] : (r[@"caption"] ? 28 : ([r[@"multi"] boolValue] ? 96 : 44));
        if ([r[@"stretch"] boolValue] || [r[@"multi"] boolValue]) h = MAX(44,self.bounds.size.height-y-54);
        UIView *v, *prior = i < (NSInteger)self.lines.count ? self.lines[i] : nil;
        if (r[@"field"]) {
            v = [self input:r index:i]; [keep addObject:r[@"field"]];
            if (r[@"tokens"] && [v isKindOfClass:UITextView.class] && !(i<(NSInteger)old.count && [old[i] isEqual:r])) {
                UITextView *code=(id)v;
                if (!code.markedTextRange && [code.text isEqual:r[@"value"]]) {
                    NSTextStorage *storage=code.textStorage; [storage beginEditing];
                    [storage addAttribute:NSForegroundColorAttributeName value:UIColor.labelColor range:NSMakeRange(0,code.text.length)];
                    for (NSArray *t in r[@"tokens"]) {
                        UIColor *color=[UIColor colorWithRed:[t[2] doubleValue] green:[t[3] doubleValue] blue:[t[4] doubleValue] alpha:1];
                        [storage addAttribute:NSForegroundColorAttributeName value:color range:NSMakeRange([t[0] unsignedIntegerValue],[t[1] unsignedIntegerValue])];
                    }
                    [storage endEditing];
                }
            }
        }
        else if (r[@"control"]) { v = [self control:r width:w height:h]; [keep addObject:r[@"control"]]; }
        else if (i < (NSInteger)old.count && [old[i] isEqual:r] && prior.frame.size.width == w) v = prior;
        else if (r[@"bar"]) v = [self cardBar:r width:w height:h];
        else if (r[@"swatches"]) v = [self swatches:r width:w height:h];
        else if (r[@"children"]) v = [self valueRow:r width:w height:h];
        else if (r[@"header"] && r[@"space"]) v = [self documentHeader:r width:w height:h];
        else if (r[@"header"]) {
            UIView *header = [UIView new]; BOOL source = [r[@"source"] boolValue];
            UILabel *label = [UILabel new]; label.text = r[@"header"]; label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleHeadline];
            label.frame = CGRectMake(source ? 48 : (self.card ? 0 : 4),0,w-(source ? 96 : 52),h); label.textAlignment = source ? NSTextAlignmentCenter : NSTextAlignmentLeft;
            [header addSubview:label]; NSDictionary *item = [r[@"items"] firstObject];
            UIButton *close = [UIButton buttonWithType:UIButtonTypeSystem]; [close setImage:[UIImage systemImageNamed:source ? @"checkmark.circle.fill" : @"xmark.circle.fill"] forState:UIControlStateNormal];
            close.tintColor = source ? self.tintColor : UIColor.tertiaryLabelColor;
            [close setPreferredSymbolConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:source ? 33 : 23] forImageInState:UIControlStateNormal];
            close.accessibilityIdentifier = [@"quiet.close." stringByAppendingString:self.key];
            close.frame = CGRectMake(w-(self.card ? 34 : 44),(h-44)/2,44,44); close.accessibilityLabel = item[@"title"];
            [header addSubview:close]; [close addAction:[self action:item owner:close] forControlEvents:UIControlEventTouchUpInside]; v = header;
            // 卡片的 ✕ 左边还可以有别的钮（表情的「随机」「移除」）。
            CGFloat right = w-38;
            for (NSDictionary *more in self.card && [r[@"items"] count] > 1 ? [r[@"items"] subarrayWithRange:NSMakeRange(1,[r[@"items"] count]-1)] : @[]) {
                UIButton *b = [self text:more]; CGFloat span = MAX(44,[b sizeThatFits:CGSizeMake(120,h)].width+16); right -= span; b.frame = CGRectMake(right,0,span,h); [header addSubview:b];
            }
        } else if ([r[@"style"] isEqual:@"tabs"]) {
            // 换分段时沿用原来的分段控件，iOS 26 的选中镜片才能一路滑过去。
            NSArray *items = r[@"items"]; UISegmentedControl *tabs = [prior isKindOfClass:UISegmentedControl.class] && ((UISegmentedControl *)prior).numberOfSegments == items.count ? (id)prior : nil;
            BOOL reuse = tabs != nil; if (!reuse) tabs = [UISegmentedControl new];
            for (NSUInteger j=0;j<items.count;j++) {
                NSDictionary *item = items[j]; UIAction *action = [self action:item owner:tabs];
                if (reuse) [tabs setAction:action forSegmentAtIndex:j]; else [tabs insertSegmentWithAction:action atIndex:j animated:NO];
                if (![item[@"on"] boolValue]) continue;
                if (tabs.selectedSegmentIndex != (NSInteger)j) tabs.selectedSegmentIndex = j;
                switched = reuse && self.tab != (NSInteger)j; self.tab = j;
            }
            v = tabs;
        } else if (r[@"caption"]) {
            UILabel *label = [UILabel new]; label.text = r[@"caption"]; label.textColor = UIColor.secondaryLabelColor;
            label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleFootnote]; v = label;
        } else v = [self line:r[@"items"] width:w height:h];
        if (prior) {
            if (prior != v && ![self.fields.allValues containsObject:prior]) {
                if ([prior isKindOfClass:UIScrollView.class] && [v isKindOfClass:UIScrollView.class])
                    ((UIScrollView *)v).contentOffset = ((UIScrollView *)prior).contentOffset;
                [prior removeFromSuperview];
            }
            self.lines[i] = v;
        } else [self.lines addObject:v];
        if (!v.superview) { [self.scroll addSubview:v]; if (below) [fresh addObject:v]; }
        if ([r[@"style"] isEqual:@"tabs"]) below = YES;
        if ([r[@"multi"] boolValue]) {
            v.layer.cornerRadius=0; v.frame=CGRectMake(0,y,self.bounds.size.width,h);
        } else if (r[@"field"] && self.sheet) v.frame=CGRectMake(x,y+3,w,h-6);
        else if (r[@"label"]) v.frame=CGRectMake(x,y,w,h);
        else if ([r[@"style"] isEqual:@"tabs"] && self.card) v.frame=CGRectMake(x,y,w,h-8);
        else if (r[@"field"] && self.card) v.frame=CGRectMake(x,y,w,h-4);
        else if ([r[@"style"] isEqual:@"tabs"] && self.sheet) v.frame=CGRectMake(x,y+2,w,h-12);
        else if (r[@"caption"] && self.sheet) v.frame=CGRectMake(x+8,y,w-8,h);
        else v.frame = CGRectMake(x,y,w,h);
        y += h;
    }
    while ((NSInteger)self.lines.count > self.listStart) {
        UIView *v = self.lines.lastObject;
        if (![self.fields.allValues containsObject:v]) [v removeFromSuperview]; [self.lines removeLastObject];
    }
    for (NSString *key in self.fields.allKeys) if (![keep containsObject:key]) {
        BOOL focused = self.fields[key].isFirstResponder;
        [self.fields[key] removeFromSuperview]; [self.fields removeObjectForKey:key];
        if (focused && keep.count) for (NSString *next in keep) if ([self.fields[next] conformsToProtocol:@protocol(UITextInput)]) { [self.fields[next] becomeFirstResponder]; break; }
    }
    if (self.sheet && (switched || self.adopted) && old) {
        // 换分段：分段控件以下的新内容淡入，列表交叉淡变。
        for (UIView *v in fresh) { v.alpha = 0; [UIView animateWithDuration:0.22 animations:^{ v.alpha = 1; }]; }
        if (self.list) { [UIView transitionWithView:self.list duration:0.22 options:UIViewAnimationOptionTransitionCrossDissolve|UIViewAnimationOptionAllowUserInteraction animations:nil completion:nil]; self.list.contentOffset = CGPointZero; self.activeRow = NSNotFound; }
    }
    self.adopted = NO;
    if (listed) {
        if (!self.list) {
            self.list = [[UITableView alloc] initWithFrame:CGRectZero style:UITableViewStylePlain];
            self.list.dataSource = self; self.list.delegate = self;
            self.list.separatorStyle = UITableViewCellSeparatorStyleNone; self.list.backgroundColor = UIColor.clearColor;
            self.list.allowsSelection = NO; self.list.contentInsetAdjustmentBehavior = UIScrollViewContentInsetAdjustmentNever;
            self.list.estimatedRowHeight = 0;
            [self.scroll addSubview:self.list]; self.activeRow = NSNotFound;
        }
        self.list.frame = CGRectMake(x,y,w,MAX(0,self.bounds.size.height-y-6));
        self.list.contentInset=UIEdgeInsetsMake(0,0,self.safeAreaInsets.bottom,0); [self.list reloadData];
        NSInteger active = NSNotFound;
        for (NSInteger i = self.listStart; i < (NSInteger)rows.count; ++i) {
            if ([rows[i][@"on"] boolValue] || [[rows[i][@"items"] firstObject][@"on"] boolValue]) { active = i-self.listStart; break; }
        }
        NSDictionary *first = self.listStart < (NSInteger)rows.count ? rows[self.listStart] : nil;
        if (active != NSNotFound && active != self.activeRow)
            [self.list scrollToRowAtIndexPath:[NSIndexPath indexPathForRow:active inSection:0] atScrollPosition:UITableViewScrollPositionNone animated:NO];
        else if (![self.key isEqual:@"catalogue"] && ![first isEqual:self.firstOption]) self.list.contentOffset = CGPointZero;
        self.activeRow = active; self.firstOption = first; self.scroll.contentSize = self.bounds.size;
    } else self.scroll.contentSize = CGSizeMake(self.bounds.size.width,y+6);
}
- (NSInteger)tableView:(UITableView *)table numberOfRowsInSection:(NSInteger)section {
    return (NSInteger)self.rows.count-self.listStart;
}
- (CGFloat)tableView:(UITableView *)table heightForRowAtIndexPath:(NSIndexPath *)path {
    NSDictionary *row = self.rows[self.listStart + path.row];
    if (row[@"empty"]) return MAX(200,table.bounds.size.height*0.62);
    if (row[@"media"]) return floor((table.bounds.size.width-10)/2*0.75)+10;
    if (row[@"snippet"]) return ceil([[self snippet:row] sizeThatFits:CGSizeMake(table.bounds.size.width-48,CGFLOAT_MAX)].height)+26;
    return row[@"h"] ? [row[@"h"] doubleValue] : 44;
}
- (UILabel *)snippet:(NSDictionary *)row {
    NSString *text = row[@"snippet"]; UIFont *font = [UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline];
    NSMutableAttributedString *body = [[NSMutableAttributedString alloc] initWithString:text attributes:@{NSFontAttributeName:font,NSForegroundColorAttributeName:UIColor.secondaryLabelColor}];
    NSArray *match = row[@"match"]; NSRange range = match.count==2 ? NSMakeRange([match[0] unsignedIntegerValue],[match[1] unsignedIntegerValue]) : NSMakeRange(0,0);
    if (range.length && NSMaxRange(range)<=text.length) [body addAttributes:@{NSFontAttributeName:[UIFont systemFontOfSize:font.pointSize weight:UIFontWeightSemibold],NSForegroundColorAttributeName:UIColor.labelColor} range:range];
    UILabel *label = [UILabel new]; label.attributedText = body; label.numberOfLines = 2; label.lineBreakMode = NSLineBreakByTruncatingTail; label.userInteractionEnabled = NO;
    return label;
}
- (UIButton *)card:(NSDictionary *)row frame:(CGRect)frame {
    // 查找结果：浅灰圆角卡片，段落图标对齐第一行，最多两行，命中处加粗变深。
    UIButton *card = [UIButton buttonWithType:UIButtonTypeCustom]; card.frame = frame;
    card.backgroundColor = [row[@"on"] boolValue] ? UIColor.tertiarySystemFillColor : UIColor.secondarySystemBackgroundColor;
    card.layer.cornerRadius = 12; card.layer.cornerCurve = kCACornerCurveContinuous;
    UILabel *label = [self snippet:row]; label.frame = CGRectMake(36,10,frame.size.width-48,frame.size.height-20); [card addSubview:label];
    UIImageView *glyph = [[UIImageView alloc] initWithImage:[UIImage systemImageNamed:row[@"symbol"] ?: @"text.alignleft" withConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:13 weight:UIImageSymbolWeightMedium]]];
    glyph.tintColor = UIColor.tertiaryLabelColor; glyph.contentMode = UIViewContentModeCenter; glyph.userInteractionEnabled = NO;
    glyph.frame = CGRectMake(10,10,20,[UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline].lineHeight); [card addSubview:glyph];
    card.accessibilityLabel = row[@"snippet"]; if ([row[@"on"] boolValue]) card.accessibilityTraits |= UIAccessibilityTraitSelected;
    [card addAction:[self action:row owner:card] forControlEvents:UIControlEventTouchUpInside];
    [card addAction:[UIAction actionWithHandler:^(UIAction *a) { ((UIView *)a.sender).alpha = 0.55; }] forControlEvents:UIControlEventTouchDown|UIControlEventTouchDragEnter];
    [card addAction:[UIAction actionWithHandler:^(UIAction *a) { [UIView animateWithDuration:0.2 animations:^{ ((UIView *)a.sender).alpha = 1; }]; }] forControlEvents:UIControlEventTouchUpInside|UIControlEventTouchUpOutside|UIControlEventTouchCancel|UIControlEventTouchDragExit];
    return card;
}
- (UIView *)tile:(NSDictionary *)item frame:(CGRect)frame {
    UIButton *tile = [UIButton buttonWithType:UIButtonTypeCustom]; tile.frame = frame;
    tile.backgroundColor = UIColor.secondarySystemBackgroundColor; tile.layer.cornerRadius = 12; tile.layer.cornerCurve = kCACornerCurveContinuous; tile.clipsToBounds = YES;
    UIImageView *image = [[UIImageView alloc] initWithFrame:tile.bounds]; image.contentMode = UIViewContentModeScaleAspectFill; image.userInteractionEnabled = NO;
    NSString *path = item[@"path"];
    if (path.length) thumbnail(image,path,MAX(frame.size.width,frame.size.height)*self.traitCollection.displayScale);
    else { image.image = [UIImage systemImageNamed:@"photo" withConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:22]]; image.tintColor = UIColor.tertiaryLabelColor; image.contentMode = UIViewContentModeCenter; }
    [tile addSubview:image]; tile.accessibilityLabel = item[@"title"];
    [tile addAction:[self action:item owner:tile] forControlEvents:UIControlEventTouchUpInside];
    return tile;
}
- (UITableViewCell *)tableView:(UITableView *)table cellForRowAtIndexPath:(NSIndexPath *)path {
    UITableViewCell *cell = [table dequeueReusableCellWithIdentifier:@"option"];
    if (!cell) cell = [[UITableViewCell alloc] initWithStyle:UITableViewCellStyleDefault reuseIdentifier:@"option"];
    cell.backgroundColor = UIColor.clearColor;
    for (UIView *old in cell.contentView.subviews.copy) if (old.tag == 101) [old removeFromSuperview];
    NSDictionary *row = self.rows[self.listStart + path.row];
    CGFloat W = table.bounds.size.width, h = [self tableView:table heightForRowAtIndexPath:path];
    UIView *v;
    if (row[@"caption"]) {
        UILabel *label = [UILabel new]; label.text = row[@"caption"]; label.textColor = UIColor.secondaryLabelColor;
        label.font = [UIFont preferredFontForTextStyle:self.sheet ? UIFontTextStyleFootnote : UIFontTextStyleBody];
        label.frame = CGRectMake(self.sheet ? 8 : 16,0,W-16,h); v = label;
    } else if (row[@"group"]) {
        UILabel *label = [UILabel new]; label.text = row[@"group"]; label.textColor = UIColor.labelColor;
        UIFont *base = [UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline];
        label.font = [UIFont systemFontOfSize:base.pointSize weight:UIFontWeightSemibold]; label.frame = CGRectMake(8,h-30,W-16,24); v = label;
    } else if (row[@"snippet"]) v = [self card:row frame:CGRectMake(0,3,W,h-6)];
    else if (row[@"empty"]) {
        UIView *box = [[UIView alloc] initWithFrame:CGRectMake(0,0,W,h)];
        UIImageView *glyph = [[UIImageView alloc] initWithImage:[UIImage systemImageNamed:row[@"symbol"] withConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:34 weight:UIImageSymbolWeightLight]]];
        glyph.tintColor = UIColor.quaternaryLabelColor; glyph.contentMode = UIViewContentModeCenter; glyph.frame = CGRectMake(0,h*0.42-56,W,48); [box addSubview:glyph];
        UILabel *label = [UILabel new]; label.text = row[@"empty"]; label.textColor = UIColor.secondaryLabelColor; label.numberOfLines = 0;
        label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline]; label.textAlignment = NSTextAlignmentCenter;
        label.frame = CGRectMake(24,h*0.42,W-48,44); [box addSubview:label]; v = box;
    } else if (row[@"media"]) {
        UIView *box = [[UIView alloc] initWithFrame:CGRectMake(0,0,W,h)]; CGFloat side = (W-10)/2;
        NSArray *items = row[@"media"];
        for (NSUInteger j = 0; j < items.count && j < 2; ++j) [box addSubview:[self tile:items[j] frame:CGRectMake(j*(side+10),0,side,h-10)]];
        v = box;
    } else {
        NSArray *items = row[@"items"];
        if ([[items firstObject][@"outline"] boolValue] && [[items firstObject][@"on"] boolValue]) {
            UIView *mark = [[UIView alloc] initWithFrame:CGRectMake(0,2,W,h-4)]; mark.tag = 101;
            mark.backgroundColor = UIColor.tertiarySystemFillColor; mark.layer.cornerRadius = 10; mark.layer.cornerCurve = kCACornerCurveContinuous;
            [cell.contentView addSubview:mark];
        }
        CGFloat indent = MIN([row[@"indent"] doubleValue],W/3);
        UIView *line = [self line:items width:W-indent height:h];
        line.frame = CGRectMake(indent,0,W-indent,h); v = line;
    }
    v.tag = 101; [cell.contentView addSubview:v];
    return cell;
}
// 下拉收起：从标题区往下拖，或列表已在顶端时往下拖，纸跟手下移；松手够远或够快就关闭，否则弹回。
- (BOOL)gestureRecognizerShouldBegin:(UIGestureRecognizer *)g {
    if (g != self.pull) return [super gestureRecognizerShouldBegin:g];
    // 卡片里正在拖的是格子、滚轮、日历或表情，不当成往下收。
    if (self.card) for (NSDictionary *r in self.rows) if (r[@"control"] && ![r[@"kind"] isEqual:@"switch"]) {
        UIView *v = self.fields[r[@"control"]]; CGPoint at = [self.pull locationInView:v], moved = [self.pull translationInView:v]; at.x -= moved.x; at.y -= moved.y;
        if ([r[@"kind"] isEqual:@"size"]) v = [v viewWithTag:5] ?: v, at = [v convertPoint:at fromView:self.fields[r[@"control"]]];
        if ([v pointInside:at withEvent:nil]) return NO;
    }
    CGPoint speed = [self.pull velocityInView:self];
    if (speed.y <= 0 || fabs(speed.x) > speed.y) return NO;
    UIView *list = self.list.superview ? self.list : nil;
    self.fromList = list && [list pointInside:[self.pull locationInView:list] withEvent:nil];
    return !self.fromList || self.list.contentOffset.y <= -self.list.adjustedContentInset.top+1;
}
- (BOOL)gestureRecognizer:(UIGestureRecognizer *)g shouldRecognizeSimultaneouslyWithGestureRecognizer:(UIGestureRecognizer *)other {
    return g == self.pull && other == self.list.panGestureRecognizer;
}
- (void)settle {
    [UIView animateWithSpringDuration:0.45 bounce:0.12 initialSpringVelocity:0 delay:0 options:UIViewAnimationOptionAllowUserInteraction|UIViewAnimationOptionBeginFromCurrentState animations:^{
        self.transform = CGAffineTransformIdentity; self.backdrop.alpha = 1;
    } completion:nil];
}
- (void)pulling:(UIPanGestureRecognizer *)g {
    CGFloat dy = [g translationInView:self.superview].y, h = MAX(1,self.bounds.size.height);
    if (g.state == UIGestureRecognizerStateBegan || g.state == UIGestureRecognizerStateChanged) {
        CGFloat t = dy > 0 ? dy : -pow(-dy,0.7);
        if (self.fromList) self.list.contentOffset = CGPointMake(0,-self.list.adjustedContentInset.top);
        self.transform = CGAffineTransformMakeTranslation(0,t); self.backdrop.alpha = 1-MAX(0,t)/h;
        return;
    }
    CGFloat speed = [g velocityInView:self.superview].y;
    if (g.state == UIGestureRecognizerStateEnded && (dy > 110 || (speed > 900 && dy > 16))) {
        self.pulled = YES; [self tappedBackdrop];
        __weak QuietEditorSurface *weak = self;
        // Rust 关掉面板后由 hide 接着滑下去；没关掉（比如正在拼音输入）就弹回。
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW,(int64_t)(0.35*NSEC_PER_SEC)),dispatch_get_main_queue(),^{
            if (weak.pulled && weak.userInteractionEnabled) { weak.pulled = NO; [weak settle]; }
        });
    } else [self settle];
}
@end

static NSMutableDictionary<NSString *, QuietEditorSurface *> *surfaces;
void quiet_editor_surface(UIView *parent, const char *key, const char *model, double x, double y, double width, double height, bool dark) {
    if (@available(iOS 26.0,*)) {
        if (!surfaces) surfaces = [NSMutableDictionary dictionary];
        NSString *name = @(key); QuietEditorSurface *v = surfaces[name]; BOOL rise = NO;
        NSString *sibling = [name isEqual:@"find"] ? @"catalogue" : ([name isEqual:@"catalogue"] ? @"find" : nil);
        if (!v && sibling && surfaces[sibling]) {
            // 目录和查找之间切换：接过同一张纸，旧 key 下一帧再 hide 时已经找不到它。
            v = surfaces[sibling]; [surfaces removeObjectForKey:sibling]; surfaces[name] = v;
            v.key = name; v.accessibilityIdentifier = [@"quiet.editor." stringByAppendingString:name]; v.adopted = YES;
        }
        if (!v) {
            BOOL page = [@[@"find",@"catalogue",@"language",@"equation"] containsObject:name], sheet = sheetKey(name), card = cardKey(name);
            v = [[QuietEditorSurface alloc] initWithEffect:page ? nil : [UIGlassEffect effectWithStyle:UIGlassEffectStyleRegular]];
            v.sheet = sheet; v.card = card; v.inset = card ? 20 : (sheet ? 16 : 10); rise = sheet || card;
            if (card) {
                // 和「文档信息」的卡片一样垫一层纸色：底下先压暗过，字在玻璃上才清楚。
                UIView *wash = [[UIView alloc] initWithFrame:CGRectZero]; wash.tag = 'wash'; wash.userInteractionEnabled = NO; wash.layer.cornerRadius = 30; wash.layer.cornerCurve = kCACornerCurveContinuous;
                wash.backgroundColor = [UIColor colorWithDynamicProvider:^UIColor *(UITraitCollection *t) { return [(t.userInterfaceStyle == UIUserInterfaceStyleDark ? UIColor.blackColor : UIColor.whiteColor) colorWithAlphaComponent:t.userInterfaceStyle == UIUserInterfaceStyleDark ? 0.32 : 0.38]; }];
                [v.contentView addSubview:wash];
            }
            if (page) { v.backgroundColor = UIColor.systemBackgroundColor; v.layer.cornerRadius = sheet ? 34 : 22; v.layer.cornerCurve = kCACornerCurveContinuous; v.layer.maskedCorners = kCALayerMinXMinYCorner | kCALayerMaxXMinYCorner; }
            if (page) {
                // 纸的底边停在键盘顶上；iOS 26 的键盘四角是圆的，往下垫一截同色的纸，圆角外露出的就是纸而不是后面压暗的页面（照 Craft）。
                // 不裁剪纸本身（内容在自己裁剪的滚动视图里），顶上的圆角由背景的 cornerRadius 画出。
                UIView *skirt = [UIView new]; skirt.tag = 'skrt';
                skirt.backgroundColor = UIColor.systemBackgroundColor; skirt.userInteractionEnabled = NO;
                [v.contentView addSubview:skirt];
            }
            if (page || card) {
                UIControl *backdrop = [[UIControl alloc] initWithFrame:parent.bounds];
                backdrop.autoresizingMask = UIViewAutoresizingFlexibleWidth|UIViewAutoresizingFlexibleHeight;
                backdrop.backgroundColor=[UIColor.blackColor colorWithAlphaComponent:card ? (dark ? 0.4 : 0.18) : (sheet ? 0.2 : 0.26)];
                [backdrop addGestureRecognizer:[[UITapGestureRecognizer alloc] initWithTarget:v action:@selector(tappedBackdrop)]];
                [parent addSubview:backdrop]; v.backdrop=backdrop;
            }
            v.key = name; v.accessibilityIdentifier = [@"quiet.editor." stringByAppendingString:name]; v.fields = [NSMutableDictionary dictionary]; v.lastFields = [NSMutableDictionary dictionary]; v.lines = [NSMutableArray array];
            if (!page) v.cornerConfiguration = [UICornerConfiguration configurationWithUniformRadius:[UICornerRadius fixedRadius:card ? 30 : ([name hasSuffix:@"bar"] ? 24 : 22)]];
            v.scroll = [UIScrollView new]; v.scroll.keyboardDismissMode = UIScrollViewKeyboardDismissModeNone;
            v.scroll.contentInsetAdjustmentBehavior = UIScrollViewContentInsetAdjustmentNever;
            if (card) { v.scroll.delaysContentTouches = NO; v.scroll.showsVerticalScrollIndicator = NO; }
            if (sheet || card) { v.pull = [[UIPanGestureRecognizer alloc] initWithTarget:v action:@selector(pulling:)]; v.pull.delegate = v; [v addGestureRecognizer:v.pull]; }
            [v.contentView addSubview:v.scroll]; [parent addSubview:v]; surfaces[name] = v;
        }
        // 用 bounds/center 摆位：升起、下拉时 transform 不是单位矩阵，frame 读回来的是变换后的值。
        BOOL resized = !CGSizeEqualToSize(v.bounds.size,CGSizeMake(width,height));
        // 卡片换内容、让开键盘时高矮跟着动，不是跳。
        BOOL glide = v.card && resized && !rise;
        CGPoint center = CGPointMake(x+width/2,y+height/2);
        if (glide) [UIView animateWithSpringDuration:0.35 bounce:0 initialSpringVelocity:0 delay:0 options:UIViewAnimationOptionAllowUserInteraction|UIViewAnimationOptionBeginFromCurrentState animations:^{
            v.bounds = CGRectMake(0,0,width,height); v.scroll.frame = v.bounds; v.center = center; [v.contentView viewWithTag:'wash'].frame = v.bounds;
        } completion:nil];
        else if (resized) { v.bounds = CGRectMake(0,0,width,height); v.scroll.frame = v.bounds; [v.contentView viewWithTag:'wash'].frame = v.bounds; [v.contentView viewWithTag:'skrt'].frame = CGRectMake(0,height,width,1200); }
        if (!CGPointEqualToPoint(v.center,center)) v.center = center;
        UIUserInterfaceStyle style = dark ? UIUserInterfaceStyleDark : UIUserInterfaceStyleLight;
        if (v.overrideUserInterfaceStyle != style) v.overrideUserInterfaceStyle = style;
        NSString *data = @(model);
        if (resized && [name isEqual:@"equation"]) {
            UIBezierPath *path = [UIBezierPath bezierPathWithRect:v.bounds];
            [path appendPath:[UIBezierPath bezierPathWithRect:CGRectMake(0,62,width,118)]];
            CAShapeLayer *mask = [CAShapeLayer layer]; mask.path = path.CGPath; mask.fillRule = kCAFillRuleEvenOdd; v.layer.mask = mask;
        }
        if (resized || ![v.model isEqual:data]) {
            NSArray *rows = [NSJSONSerialization JSONObjectWithData:[data dataUsingEncoding:NSUTF8StringEncoding] options:0 error:NULL];
            if (rows) [v update:rows width:MAX(1,width-2*v.inset)]; v.model = data;
        }
        if (rise) {
            v.transform = CGAffineTransformMakeTranslation(0,height+40); v.backdrop.alpha = 0;
            [UIView animateWithSpringDuration:0.5 bounce:0.06 initialSpringVelocity:0 delay:0 options:UIViewAnimationOptionAllowUserInteraction animations:^{
                v.transform = CGAffineTransformIdentity; v.backdrop.alpha = 1;
            } completion:nil];
        }
    }
}
void quiet_hide_editor_surface(const char *key) {
    NSString *name = @(key); QuietEditorSurface *v = surfaces[name]; if (!v) return;
    [surfaces removeObjectForKey:name]; UIControl *backdrop = v.backdrop;
    if (!(v.sheet || v.card) || !v.window) { [backdrop removeFromSuperview]; [v removeFromSuperview]; return; }
    // 收起：先交出焦点，纸从当前位置滑到底下，压暗同时淡出，结束后再移除。
    v.userInteractionEnabled = NO; backdrop.userInteractionEnabled = NO; v.pulled = NO; [v endEditing:YES];
    [UIView animateWithSpringDuration:0.4 bounce:0 initialSpringVelocity:0 delay:0 options:UIViewAnimationOptionBeginFromCurrentState animations:^{
        v.transform = CGAffineTransformMakeTranslation(0,v.bounds.size.height+40); backdrop.alpha = 0;
    } completion:^(__unused BOOL done) { [backdrop removeFromSuperview]; [v removeFromSuperview]; }];
}
