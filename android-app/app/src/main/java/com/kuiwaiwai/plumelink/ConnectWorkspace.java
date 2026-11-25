package com.kuiwaiwai.plumelink;

import android.content.Context;
import android.graphics.drawable.GradientDrawable;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.*;

final class ConnectWorkspace extends LinearLayout {
    private final LinearLayout panel, dock, dockMeta;
    private final ScrollView content, actions;
    private final ImageView logo;
    private final TextView title, subtitle;
    private final GradientDrawable dockBackground = new GradientDrawable();
    private AdaptiveLayout.Connection mode;
    private Boolean compact;
    private int railWidth;

    ConnectWorkspace(Context context, LinearLayout panel, ScrollView content, LinearLayout dock,
                     LinearLayout dockMeta, ImageView logo, TextView title, TextView subtitle, int surface) {
        super(context);
        this.panel=panel;this.content=content;this.dock=dock;this.dockMeta=dockMeta;
        this.logo=logo;this.title=title;this.subtitle=subtitle;
        dockBackground.setColor(surface);
        setTag("connection-workspace");content.setTag("connection-list-scroll");dock.setTag("connection-dock");
        actions=new ScrollView(context);actions.setVerticalScrollBarEnabled(false);actions.addView(dock);
        addView(content);addView(actions);
    }
    private int dp(int value) {return Math.round(value*getResources().getDisplayMetrics().density);}
    @Override protected void onMeasure(int widthSpec,int heightSpec) {
        float density=getResources().getDisplayMetrics().density;
        int width=Math.round(MeasureSpec.getSize(widthSpec)/density), height=Math.round(MeasureSpec.getSize(heightSpec)/density);
        AdaptiveLayout.Connection next=AdaptiveLayout.connection(width,height);
        boolean shortHeight=height<480;
        int rail=dp(AdaptiveLayout.connectionRailDp(width));
        if(mode!=next || railWidth!=rail) {
            mode=next;railWidth=rail;
            boolean side=mode==AdaptiveLayout.Connection.ACTION_RAIL, flow=mode==AdaptiveLayout.Connection.SCROLLING;
            setOrientation(side?HORIZONTAL:VERTICAL);
            ViewGroup target=flow?panel:actions;
            if(dock.getParent()!=target) {((ViewGroup)dock.getParent()).removeView(dock);target.addView(dock,new ViewGroup.LayoutParams(-1,-2));}
            actions.setVisibility(flow?GONE:VISIBLE);actions.setFillViewport(side);
            content.setLayoutParams(new LinearLayout.LayoutParams(side?0:-1,side?-1:0,1));
            actions.setLayoutParams(new LinearLayout.LayoutParams(side?rail:-1,side?-1:-2));
            dock.setGravity(side?Gravity.CENTER_VERTICAL:Gravity.TOP);
            dockMeta.setOrientation(side?VERTICAL:HORIZONTAL);
            for(int i=0;i<dockMeta.getChildCount();i++)dockMeta.getChildAt(i).setLayoutParams(new LinearLayout.LayoutParams(side?-1:0,-2,side?0:1));
            dockBackground.setCornerRadius(flow?dp(16):0);dock.setBackground(dockBackground);
            dock.setElevation(flow||side?0:dp(4));
        }
        if(compact==null || compact!=shortHeight) {
            compact=shortHeight;
            panel.setPadding(dp(shortHeight?16:20),dp(shortHeight?12:16),dp(shortHeight?16:20),dp(shortHeight?12:16));
            int size=dp(shortHeight?28:36);logo.setLayoutParams(new LinearLayout.LayoutParams(size,size));
            title.setTextSize(shortHeight?22:28);title.setPadding(0,dp(shortHeight?12:24),0,shortHeight?dp(16):0);
            subtitle.setVisibility(shortHeight?GONE:VISIBLE);
            dock.setPadding(dp(shortHeight?16:20),dp(shortHeight?12:16),dp(shortHeight?16:20),dp(shortHeight?12:16));
        }
        super.onMeasure(widthSpec,heightSpec);
    }
}
